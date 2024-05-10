use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A simple history implementation that stores values at a given index.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(bound = "T: Serialize + DeserializeOwned")]
pub struct History<T> {
    pub last_index: u64,
    pub values: Vec<(u64, T)>,
}

#[derive(Debug)]
pub enum HistoryError {
    ValueNotOrdered,
}

/// A simple history implementation that stores values at a given index.
/// It allows to get the value at a given index, push a new value with an index,
/// and revert the history to a given index.
///
/// ***Note:*** This implementation need to insert the values in order.
impl<T> History<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub fn new(index: u64, value: T) -> Self {
        Self { last_index: index, values: vec![(index, value)] }
    }

    /// Push a new value with an index.
    /// If the index is smaller or equal to the last index, it will return an error.
    pub fn push(&mut self, index: u64, value: T) -> Result<(), HistoryError> {
        if self.last_index != 0 && self.last_index >= index {
            return Err(HistoryError::ValueNotOrdered);
        }
        self.last_index = index;
        self.values.push((index, value));
        Ok(())
    }

    /// Get the value at a given index.
    /// If the index is not found, it will return the value at the previous index.
    /// If the index is smaller than the first index, it will return None.
    pub fn get_at(&self, index: u64) -> Option<&T> {
        match self.values.binary_search_by_key(&index, |&(i, _)| i) {
            Ok(i) => Some(&self.values[i].1),
            Err(0) => None,
            Err(i) => Some(&self.values[i - 1].1),
        }
    }

    /// Get the last value.
    pub fn get(&self) -> Option<&T> {
        self.values.last().map(|(_, value)| value)
    }

    /// Revert the history to a given index.
    /// If the index is not found, it will revert to the previous index.
    /// If the index is smaller than the first index, it will clear the history.
    pub fn revert_to(&mut self, index: u64) {
        match self.values.binary_search_by_key(&index, |&(i, _)| i) {
            Ok(i) => self.values.truncate(i + 1),
            Err(0) => self.values.clear(),
            Err(i) => self.values.truncate(i),
        }
        self.last_index = self.values.last().map(|(i, _)| *i).unwrap_or_default();
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl<T> std::fmt::Debug for History<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "History (last_index: {}) {{ ", self.last_index)?;
        for (index, value) in &self.values {
            write!(f, "{:?} => {:?}, ", index, value)?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history() {
        let mut history = History::<u64>::default();

        assert_eq!(history.get(), None);
        assert_eq!(history.get_at(0), None);

        history.push(0, 0).unwrap();
        assert_eq!(history.get(), Some(&0));
        assert_eq!(history.get_at(0), Some(&0));
        assert_eq!(history.get_at(1), Some(&0));

        history.push(1, 1).unwrap();
        assert_eq!(history.get(), Some(&1));
        assert_eq!(history.get_at(0), Some(&0));
        assert_eq!(history.get_at(1), Some(&1));
        assert_eq!(history.get_at(2), Some(&1));

        history.push(2, 2).unwrap();
        assert_eq!(history.get(), Some(&2));
        assert_eq!(history.get_at(0), Some(&0));
        assert_eq!(history.get_at(1), Some(&1));
        assert_eq!(history.get_at(2), Some(&2));
        assert_eq!(history.get_at(3), Some(&2));

        history.push(1, 3).unwrap_err();
        assert_eq!(history.get(), Some(&2));
        assert_eq!(history.get_at(0), Some(&0));
        assert_eq!(history.get_at(1), Some(&1));
        assert_eq!(history.get_at(2), Some(&2));
        assert_eq!(history.get_at(3), Some(&2));

        history.revert_to(1);
        assert_eq!(history.get(), Some(&1));
        assert_eq!(history.get_at(0), Some(&0));
        assert_eq!(history.get_at(1), Some(&1));
        assert_eq!(history.get_at(2), Some(&1));
    }
}
