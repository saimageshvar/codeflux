use std::collections::HashMap;

/// A string-to-u32 interning table.
///
/// Stores strings in a contiguous buffer. Each string gets a unique `u32` ID.
/// Designed for serialization: the blob + offsets can be written directly to disk.
pub struct StringTable {
    /// Maps string content to its assigned ID.
    map: HashMap<String, u32>,
    /// Ordered entries: (offset_in_blob, length).
    entries: Vec<(u32, u32)>,
    /// Contiguous UTF-8 blob containing all interned strings.
    blob: Vec<u8>,
}

impl StringTable {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            entries: Vec::new(),
            blob: Vec::new(),
        }
    }

    /// Intern a string, returning its ID. Returns existing ID if already interned.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = self.entries.len() as u32;
        let offset = self.blob.len() as u32;
        let len = s.len() as u32;
        self.blob.extend_from_slice(s.as_bytes());
        self.entries.push((offset, len));
        self.map.insert(s.to_owned(), id);
        id
    }

    /// Resolve an ID back to its string. Panics if ID is out of range.
    pub fn resolve(&self, id: u32) -> &str {
        let (offset, len) = self.entries[id as usize];
        let bytes = &self.blob[offset as usize..(offset + len) as usize];
        std::str::from_utf8(bytes).expect("interned string is valid UTF-8")
    }

    /// Number of interned strings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access the raw blob for serialization.
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// Access the entries table for serialization.
    pub fn entries(&self) -> &[(u32, u32)] {
        &self.entries
    }

    /// Look up an existing string without interning it. Returns None if not found.
    pub fn lookup(&self, s: &str) -> Option<u32> {
        self.map.get(s).copied()
    }

    /// Reconstruct a StringTable from serialized components.
    pub fn from_parts(entries: Vec<(u32, u32)>, blob: Vec<u8>) -> Self {
        let mut map = HashMap::new();
        for (i, &(offset, len)) in entries.iter().enumerate() {
            let s = std::str::from_utf8(&blob[offset as usize..(offset + len) as usize])
                .expect("valid UTF-8")
                .to_owned();
            map.insert(s, i as u32);
        }
        Self { map, entries, blob }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_and_resolve() {
        let mut table = StringTable::new();
        let id1 = table.intern("User#deactivate!");
        let id2 = table.intern("User#activate!");
        let id3 = table.intern("User#deactivate!"); // duplicate

        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
        assert_eq!(id3, id1); // same string → same ID
        assert_eq!(table.resolve(id1), "User#deactivate!");
        assert_eq!(table.resolve(id2), "User#activate!");
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_lookup() {
        let mut table = StringTable::new();
        table.intern("foo");
        assert_eq!(table.lookup("foo"), Some(0));
        assert_eq!(table.lookup("bar"), None);
    }

    #[test]
    fn test_empty_string() {
        let mut table = StringTable::new();
        let id = table.intern("");
        assert_eq!(table.resolve(id), "");
    }

    #[test]
    fn test_blob_layout() {
        let mut table = StringTable::new();
        table.intern("abc");
        table.intern("de");
        // blob should be "abcde"
        assert_eq!(table.blob(), b"abcde");
        assert_eq!(table.entries(), &[(0, 3), (3, 2)]);
    }
}
