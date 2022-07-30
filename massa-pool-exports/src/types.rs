//! Copyright (c) 2022 MASSA LABS <info@massa.net>

use std::cmp::Reverse;

use massa_models::OperationId;
use num::rational::Ratio;


type OperationCursorInner = (Reverse<Ratio<u64>>, OperationId);
/// A cursor for pool operations, sorted by increasing quality
#[derive(PartialEq, Eq, PartialOrd, Ord,  Copy, Clone)]
pub struct PoolOperationCursor((Reverse<Ratio<u64>>, OperationId));

impl PoolOperationCursor {
    pub fn new(inner: OperationCursorInner) -> Self {
        Self(inner)
    }
}