pub mod intern;
pub mod graph;
pub mod index;

/// Unique identifier for an interned method string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(pub u32);

/// Unique identifier for an interned test string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TestId(pub u32);

/// Format version for .cfx index files.
pub const CFX_VERSION: u16 = 1;
pub const CFX_MIN_READER_VERSION: u16 = 1;
pub const CFX_MAGIC: &[u8; 4] = b"CFX\0";
