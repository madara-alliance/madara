use core::marker::PhantomData;

/// A trait for types that should be serialized or deserialized as hexadecimal strings.
pub trait NumAsHex: Sized {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer;

    fn deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>;
}

impl NumAsHex for u64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        /// The symbols to be used for the hexadecimal representation.
        const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";
        /// The maximum number of digits in the hexadecimal representation of a `u64`.
        const MAX_NUMBER_SIZE: usize = u64::MAX.ilog(16) as usize + 1;

        if *self == 0 {
            return serializer.serialize_str("0x0");
        }

        // The following code can be very much optimized simply by making everything
        // `unsafe` and using pointers to write to the buffer.
        // Let's benchmark it first to ensure that it's actually worth it.

        // The buffer is filled from the end to the beginning.
        // We know that it will always have the correct size because we made it have the
        // maximum possible size for a base-16 representation of a `u64`.
        //
        // +-----------------------------------+
        // +                           1 2 f a +
        // +-----------------------------------+
        //                           ^ cursor
        //
        // Once the number has been written to the buffer, we simply add a `0x` prefix
        // to the beginning of the buffer. Just like the digits, we know the buffer is
        // large enough to hold the prefix.
        //
        // +-----------------------------------+
        // +                       0 x 1 2 f a +
        // +-----------------------------------+
        //                       ^ cursor
        // |-----------------------| remaining
        //
        // The output string is the part of the buffer that has been written. In other
        // words, we have to skip all the bytes that *were not* written yet (remaining).

        let mut buffer = [0u8; MAX_NUMBER_SIZE + 2]; // + 2 to account for 0x
        let mut cursor = buffer.iter_mut().rev();
        let mut n = *self;
        while n != 0 {
            *cursor.next().unwrap() = HEX_DIGITS[(n % 16) as usize];
            n /= 16;
        }
        *cursor.next().unwrap() = b'x';
        *cursor.next().unwrap() = b'0';

        let remaining = cursor.len();

        // SAFETY:
        //  We only wrote ASCII characters to the buffer, ensuring that it is only composed
        //  of valid UTF-8 code points. This unwrap can never fail. Just like the code above,
        //  using `from_utf8_unchecked` is safe.
        let s = core::str::from_utf8(&buffer[remaining..]).unwrap();

        serializer.serialize_str(s)
    }

    fn deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NumAsHexVisitor;

        impl serde::de::Visitor<'_> for NumAsHexVisitor {
            type Value = u64;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("a hexadecimal string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // Just like the serialization code, this can be further optimized using
                // unsafe code and pointers. Though the gain will probably be less interesting.

                // Explicitly avoid being UTF-8 aware.
                let mut bytes = v.as_bytes();

                // If the input string does not start with the `0x` prefix, then it's an
                // error. The `NUM_AS_HEX` regex defined in the specification specifies
                // this prefix as mandatory.
                bytes = bytes
                    .strip_prefix(b"0x")
                    .ok_or_else(|| E::custom("expected a hexadecimal string starting with 0x"))?;

                if bytes.is_empty() {
                    return Err(E::custom("expected a hexadecimal string"));
                }

                // Remove the leading zeros from the string, if any.
                // We need this in order to optimize the code below with the knowledge of the
                // length of the hexadecimal representation of the number.
                while let Some(rest) = bytes.strip_prefix(b"0") {
                    bytes = rest;
                }

                // If the string has a size larger than the maximum size of the hexadecimal
                // representation of a `u64`, then we're forced to overflow.
                if bytes.len() > u64::MAX.ilog(16) as usize + 1 {
                    return Err(E::custom("integer overflowed 64-bit"));
                }

                // Aggregate the digits into `n`,
                // Digits from `0` to `9` represent numbers from `0` to `9`.
                // Letters from `a` to `f` represent numbers from `10` to `15`.
                //
                // As specified in the spec, both uppercase and lowercase characters are
                // allowed.
                //
                // Because we already checked the size of the string earlier, we know that
                // the following code will never overflow.
                let mut n = 0u64;
                for &b in bytes.iter() {
                    let unit = match b {
                        b'0'..=b'9' => b as u64 - b'0' as u64,
                        b'a'..=b'f' => b as u64 - b'a' as u64 + 10,
                        b'A'..=b'F' => b as u64 - b'A' as u64 + 10,
                        _ => return Err(E::custom("invalid hexadecimal digit")),
                    };

                    n = n * 16 + unit;
                }

                Ok(n)
            }
        }

        deserializer.deserialize_str(NumAsHexVisitor)
    }
}

impl NumAsHex for u128 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        /// The symbols to be used for the hexadecimal representation.
        const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";
        /// The maximum number of digits in the hexadecimal representation of a `u64`.
        const MAX_NUMBER_SIZE: usize = u128::MAX.ilog(16) as usize + 1;

        if *self == 0 {
            return serializer.serialize_str("0x0");
        }

        let mut buffer = [0u8; MAX_NUMBER_SIZE + 2]; // + 2 to account for 0x
        let mut cursor = buffer.iter_mut().rev();
        let mut n = *self;
        while n != 0 {
            *cursor.next().unwrap() = HEX_DIGITS[(n % 16) as usize];
            n /= 16;
        }
        *cursor.next().unwrap() = b'x';
        *cursor.next().unwrap() = b'0';

        let remaining = cursor.len();

        // SAFETY:
        //  We only wrote ASCII characters to the buffer, ensuring that it is only composed
        //  of valid UTF-8 code points. This unwrap can never fail. Just like the code above,
        //  using `from_utf8_unchecked` is safe.
        let s = core::str::from_utf8(&buffer[remaining..]).unwrap();

        serializer.serialize_str(s)
    }

    fn deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NumAsHexVisitor;

        impl serde::de::Visitor<'_> for NumAsHexVisitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("a hexadecimal string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // Explicitly avoid being UTF-8 aware.
                let mut bytes = v.as_bytes();

                // If the input string does not start with the `0x` prefix, then it's an
                // error. The `NUM_AS_HEX` regex defined in the specification specifies
                // this prefix as mandatory.
                bytes = bytes
                    .strip_prefix(b"0x")
                    .ok_or_else(|| E::custom("expected a hexadecimal string starting with 0x"))?;

                if bytes.is_empty() {
                    return Err(E::custom("expected a hexadecimal string"));
                }

                // Remove the leading zeros from the string, if any.
                // We need this in order to optimize the code below with the knowledge of the
                // length of the hexadecimal representation of the number.
                while let Some(rest) = bytes.strip_prefix(b"0") {
                    bytes = rest;
                }

                // If the string has a size larger than the maximum size of the hexadecimal
                // representation of a `u64`, then we're forced to overflow.
                if bytes.len() > u128::MAX.ilog(16) as usize + 1 {
                    return Err(E::custom("integer overflowed 64-bit"));
                }

                // Aggregate the digits into `n`,
                // Digits from `0` to `9` represent numbers from `0` to `9`.
                // Letters from `a` to `f` represent numbers from `10` to `15`.
                //
                // As specified in the spec, both uppercase and lowercase characters are
                // allowed.
                //
                // Because we already checked the size of the string earlier, we know that
                // the following code will never overflow.
                let mut n = 0u128;
                for &b in bytes.iter() {
                    let unit = match b {
                        b'0'..=b'9' => b as u128 - b'0' as u128,
                        b'a'..=b'f' => b as u128 - b'a' as u128 + 10,
                        b'A'..=b'F' => b as u128 - b'A' as u128 + 10,
                        _ => return Err(E::custom("invalid hexadecimal digit")),
                    };

                    n = n * 16 + unit;
                }

                Ok(n)
            }
        }

        deserializer.deserialize_str(NumAsHexVisitor)
    }
}

impl<T> NumAsHex for Option<T>
where
    T: NumAsHex,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            None => serializer.serialize_none(),
            Some(v) => v.serialize(serializer),
        }
    }

    fn deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptionVisitor<T>(PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for OptionVisitor<T>
        where
            T: NumAsHex,
        {
            type Value = Option<T>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                writeln!(formatter, "an optional number as a hexadecimal string")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(None)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                T::deserialize(deserializer).map(Some)
            }
        }

        deserializer.deserialize_option(OptionVisitor(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct Helper<T>
    where
        T: NumAsHex,
    {
        #[serde(with = "NumAsHex")]
        num: T,
    }

    fn serialize<T: NumAsHex>(num: T) -> serde_json::Result<String> {
        let helper = Helper { num };
        serde_json::to_string(&helper)
    }

    fn deserialize<T: NumAsHex>(s: &str) -> serde_json::Result<T> {
        let helper: Helper<T> = serde_json::from_str(s)?;
        Ok(helper.num)
    }

    #[test]
    #[cfg(test)]
    fn serialize_0_hex() {
        assert_eq!(serialize(0u64).unwrap(), "\"0x0\"");
        assert_eq!(serialize(0u128).unwrap(), "\"0x0\"");
    }

    #[test]
    #[cfg(test)]
    fn srialize_hex() {
        assert_eq!(serialize(0x1234u64).unwrap(), "\"0x1234\"");
        assert_eq!(serialize(0x1234u128).unwrap(), "\"0x1234\"");
    }

    #[test]
    #[cfg(test)]
    fn srialize_max() {
        assert_eq!(serialize(u64::MAX).unwrap(), "\"0xffffffffffffffff\"");
        assert_eq!(serialize(u128::MAX).unwrap(), "\"0xffffffffffffffffffffffffffffffff\"");
    }

    #[test]
    #[cfg(test)]
    fn deserialize_zero() {
        assert_eq!(deserialize::<u64>("\"0x0\"").unwrap(), 0);
        assert_eq!(deserialize::<u128>("\"0x0\"").unwrap(), 0);
    }

    #[test]
    #[cfg(test)]
    fn deserialize_zeros() {
        assert_eq!(deserialize::<u64>("\"0x00000\"").unwrap(), 0);
        assert_eq!(deserialize::<u128>("\"0x00000\"").unwrap(), 0);
    }

    #[test]
    #[cfg(test)]
    fn deserialize_max() {
        assert_eq!(deserialize::<u64>("\"0xFFFFFFFFFFFFFFFF\"").unwrap(), u64::MAX);
        assert_eq!(deserialize::<u128>("\"0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\"").unwrap(), u128::MAX);
    }

    #[test]
    #[cfg(test)]
    fn deserialize_big_one() {
        assert_eq!(deserialize::<u64>("\"0x000000000000000000000000000001\"").unwrap(), 1);
        assert_eq!(
            deserialize::<u128>("\"0x00000000000000000000000000000000000000000000000000000000001\"").unwrap(),
            1
        );
    }

    #[test]
    #[cfg(test)]
    fn deserialize_hex() {
        assert_eq!(deserialize::<u64>("\"0x1234\"").unwrap(), 0x1234);
        assert_eq!(deserialize::<u128>("\"0x1234\"").unwrap(), 0x1234);
    }
}
