// Copyright (c) 2022 MASSA LABS <info@massa.net>

use crate::error::BootstrapError;
use crate::establisher::types::Duplex;
use crate::messages::BootstrapClientMessage;
use crate::messages::BootstrapServerMessage;
use massa_hash::Hash;
use massa_hash::HASH_SIZE_BYTES;
use massa_models::Version;
use massa_models::{
    constants::BOOTSTRAP_RANDOMNESS_SIZE_BYTES, with_serialization_context, DeserializeCompact,
    DeserializeMinBEInt, SerializeCompact, SerializeMinBEInt,
};
use massa_signature::{sign, PrivateKey};
use std::convert::TryInto;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bootstrap server binder
pub struct BootstrapServerBinder {
    max_bootstrap_message_size: u32,
    size_field_len: usize,
    local_privkey: PrivateKey,
    duplex: Duplex,
    prev_message: Option<Hash>,
}

impl BootstrapServerBinder {
    /// Creates a new `WriteBinder`.
    ///
    /// # Argument
    /// * duplex: duplex stream.
    pub fn new(duplex: Duplex, local_privkey: PrivateKey) -> Self {
        let max_bootstrap_message_size =
            with_serialization_context(|context| context.max_bootstrap_message_size);
        let size_field_len = u32::be_bytes_min_length(max_bootstrap_message_size);
        BootstrapServerBinder {
            max_bootstrap_message_size,
            size_field_len,
            local_privkey,
            duplex,
            prev_message: None,
        }
    }
}

impl BootstrapServerBinder {
    /// Performs a handshake. Should be called after connection
    /// NOT cancel-safe
    /// MUST always be followed by a send of the BootstrapMessage::BootstrapTime
    pub async fn handshake(&mut self, version: Version) -> Result<(), BootstrapError> {
        // read version and random bytes, send signature
        let msg_hash = {
            let version_bytes = version.to_bytes_compact()?;
            let mut msg_bytes = vec![0u8; version_bytes.len() + BOOTSTRAP_RANDOMNESS_SIZE_BYTES];
            self.duplex.read_exact(&mut msg_bytes).await?;
            let received_version = Version::from_bytes_compact(&msg_bytes[..version_bytes.len()])?;
            if !received_version.0.is_compatible(&version) {
                return Err(BootstrapError::IncompatibleVersionError(format!("Received a bad incompatible version in handshake. (excepted: {}, received: {})", version, received_version.0)));
            }
            Hash::compute_from(&msg_bytes)
        };

        // save prev sig
        self.prev_message = Some(msg_hash);

        Ok(())
    }

    /// Writes the next message. NOT cancel-safe
    pub async fn send(&mut self, msg: BootstrapServerMessage) -> Result<(), BootstrapError> {
        // serialize message
        let msg_bytes = msg.to_bytes_compact()?;
        let msg_len: u32 = msg_bytes.len().try_into().map_err(|e| {
            BootstrapError::GeneralError(format!("bootstrap message too large to encode: {}", e))
        })?;

        // compute signature
        let sig = {
            if let Some(prev_message) = self.prev_message {
                // there was a previous message: sign(prev_msg_hash + msg)
                let mut signed_data =
                    Vec::with_capacity(HASH_SIZE_BYTES.saturating_add(msg_len as usize));
                signed_data.extend(prev_message.to_bytes());
                signed_data.extend(&msg_bytes);
                sign(&Hash::compute_from(&signed_data), &self.local_privkey)?
            } else {
                // there was no previous message: sign(msg)
                sign(&Hash::compute_from(&msg_bytes), &self.local_privkey)?
            }
        };

        // send signature
        self.duplex.write_all(sig.to_bytes()).await?;

        // send message length
        {
            let msg_len_bytes = msg_len.to_be_bytes_min(self.max_bootstrap_message_size)?;
            self.duplex.write_all(&msg_len_bytes).await?;
        }

        // send message
        self.duplex.write_all(&msg_bytes).await?;

        // save prev sig
        self.prev_message = Some(Hash::compute_from(sig.to_bytes()));

        Ok(())
    }

    #[allow(dead_code)]
    /// Read a message sent from the client (not signed). NOT cancel-safe
    pub async fn next(&mut self) -> Result<BootstrapClientMessage, BootstrapError> {
        // read prev hash
        let received_prev_hash = {
            if self.prev_message.is_some() {
                let mut hash_bytes = [0u8; HASH_SIZE_BYTES];
                self.duplex.read_exact(&mut hash_bytes).await?;
                Some(Hash::from_bytes(&hash_bytes))
            } else {
                None
            }
        };

        // read message length
        let msg_len = {
            let mut msg_len_bytes = vec![0u8; self.size_field_len];
            self.duplex.read_exact(&mut msg_len_bytes[..]).await?;
            u32::from_be_bytes_min(&msg_len_bytes, self.max_bootstrap_message_size)?.0
        };

        // read message
        let mut msg_bytes = vec![0u8; msg_len as usize];
        self.duplex.read_exact(&mut msg_bytes).await?;

        // check previous hash
        if received_prev_hash != self.prev_message {
            return Err(BootstrapError::GeneralError(
                "Message sequencing has been broken".to_string(),
            ));
        }

        // update previous hash
        if let Some(prev_hash) = received_prev_hash {
            // there was a previous message: hash(prev_hash + message)
            let mut hashed_bytes =
                Vec::with_capacity(HASH_SIZE_BYTES.saturating_add(msg_bytes.len()));
            hashed_bytes.extend(prev_hash.to_bytes());
            hashed_bytes.extend(&msg_bytes);
            self.prev_message = Some(Hash::compute_from(&hashed_bytes));
        } else {
            // no previous message: hash message only
            self.prev_message = Some(Hash::compute_from(&msg_bytes));
        }

        // deserialize message
        let (msg, _len) = BootstrapClientMessage::from_bytes_compact(&msg_bytes)?;

        Ok(msg)
    }
}