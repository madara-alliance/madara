pub fn has_dup<T: PartialEq>(slice: &[T]) -> bool {
    for i in 1..slice.len() {
        if slice[i..].contains(&slice[i - 1]) {
            return true;
        }
    }
    false
}

pub fn is_sorted<T>(data: &[T]) -> bool
where
    T: Ord,
{
    if data.len() == 1 {
        return true;
    }
    data.windows(2).all(|w| w[0] <= w[1])
}
