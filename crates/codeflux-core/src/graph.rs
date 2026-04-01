use std::collections::HashMap;
use crate::{MethodId, TestId};

/// Maps method_id → sorted list of test_ids that invoke it.
pub struct InvertedIndex {
    data: HashMap<MethodId, Vec<TestId>>,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }

    /// Record that `test` invokes `method`.
    pub fn add(&mut self, method: MethodId, test: TestId) {
        self.data.entry(method).or_default().push(test);
    }

    /// Finalize: sort and deduplicate all posting lists.
    pub fn finalize(&mut self) {
        for tests in self.data.values_mut() {
            tests.sort();
            tests.dedup();
        }
    }

    /// Get tests that invoke a given method.
    pub fn get(&self, method: MethodId) -> &[TestId] {
        self.data.get(&method).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Iterate over all (method, tests) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&MethodId, &[TestId])> {
        self.data.iter().map(|(k, v)| (k, v.as_slice()))
    }

    pub fn method_count(&self) -> usize {
        self.data.len()
    }
}

/// Maps test_id → sorted list of method_ids it invokes.
pub struct ForwardIndex {
    data: HashMap<TestId, Vec<MethodId>>,
}

impl ForwardIndex {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }

    pub fn add(&mut self, test: TestId, method: MethodId) {
        self.data.entry(test).or_default().push(method);
    }

    pub fn finalize(&mut self) {
        for methods in self.data.values_mut() {
            methods.sort();
            methods.dedup();
        }
    }

    pub fn get(&self, test: TestId) -> &[MethodId] {
        self.data.get(&test).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn test_count(&self) -> usize {
        self.data.len()
    }

    /// Iterate over all (test, methods) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&TestId, &[MethodId])> {
        self.data.iter().map(|(k, v)| (k, v.as_slice()))
    }
}

/// Maps file_path (as interned u32) → list of method_ids defined in that file.
pub struct FileMethodMap {
    data: HashMap<u32, Vec<MethodId>>,
}

impl FileMethodMap {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }

    /// Record that `method` is defined in `file`.
    pub fn add(&mut self, file_id: u32, method: MethodId) {
        self.data.entry(file_id).or_default().push(method);
    }

    pub fn finalize(&mut self) {
        for methods in self.data.values_mut() {
            methods.sort();
            methods.dedup();
        }
    }

    /// Get all methods defined in a file.
    pub fn get(&self, file_id: u32) -> &[MethodId] {
        self.data.get(&file_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &[MethodId])> {
        self.data.iter().map(|(k, v)| (k, v.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inverted_index() {
        let mut idx = InvertedIndex::new();
        idx.add(MethodId(0), TestId(1));
        idx.add(MethodId(0), TestId(2));
        idx.add(MethodId(0), TestId(1)); // duplicate
        idx.add(MethodId(1), TestId(3));
        idx.finalize();

        assert_eq!(idx.get(MethodId(0)), &[TestId(1), TestId(2)]);
        assert_eq!(idx.get(MethodId(1)), &[TestId(3)]);
        assert_eq!(idx.get(MethodId(99)), &[]); // missing
    }

    #[test]
    fn test_forward_index() {
        let mut idx = ForwardIndex::new();
        idx.add(TestId(0), MethodId(1));
        idx.add(TestId(0), MethodId(2));
        idx.finalize();

        assert_eq!(idx.get(TestId(0)), &[MethodId(1), MethodId(2)]);
        assert_eq!(idx.get(TestId(99)), &[]);
    }

    #[test]
    fn test_file_method_map() {
        let mut fmap = FileMethodMap::new();
        fmap.add(0, MethodId(1));
        fmap.add(0, MethodId(2));
        fmap.add(1, MethodId(3));
        fmap.finalize();

        assert_eq!(fmap.get(0), &[MethodId(1), MethodId(2)]);
        assert_eq!(fmap.get(1), &[MethodId(3)]);
    }
}
