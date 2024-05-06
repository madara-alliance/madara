use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A simple history implementation that stores values at a given index.
#[derive(Serialize, Deserialize, Default)]
#[serde(bound = "T: Serialize + DeserializeOwned")]
pub struct History<T>(pub Vec<(u64, T)>);

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
    /// Push a new value with an index.
    /// If the index is smaller or equal to the last index, it will return an error.
    pub fn push(&mut self, index: u64, value: T) -> Result<(), HistoryError> {
        match self.0.last() {
            Some((last_index, _)) if index <= *last_index => Err(HistoryError::ValueNotOrdered),
            _ => {
                self.0.push((index, value));
                Ok(())
            }
        }
    }

    /// Get the value at a given index.
    /// If the index is not found, it will return the value at the previous index.
    /// If the index is smaller than the first index, it will return None.
    pub fn get_at(&self, index: u64) -> Option<&T> {
        match self.0.binary_search_by_key(&index, |&(i, _)| i) {
            Ok(i) => Some(&self.0[i].1),
            Err(0) => None,
            Err(i) => Some(&self.0[i - 1].1),
        }
    }

    /// Get the last value.
    pub fn get(&self) -> Option<&T> {
        self.0.last().map(|(_, value)| value)
    }

    /// Revert the history to a given index.
    /// If the index is not found, it will revert to the previous index.
    /// If the index is smaller than the first index, it will clear the history.
    pub fn revert_to(&mut self, index: u64) {
        match self.0.binary_search_by_key(&index, |&(i, _)| i) {
            Ok(i) => self.0.truncate(i + 1),
            Err(0) => self.0.clear(),
            Err(i) => self.0.truncate(i),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> std::fmt::Debug for History<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "History {{ ")?;
        for (index, value) in &self.0 {
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
