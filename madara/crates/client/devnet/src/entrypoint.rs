use std::ops::Deref;

use starknet::core::{types::Felt, utils::starknet_keccak};

pub struct Call {
    pub to: Felt,
    pub selector: Selector,
    pub calldata: Vec<Felt>,
}
impl Call {
    pub fn flatten(&self) -> impl Iterator<Item = Felt> + '_ {
        [self.to, self.selector.0, self.calldata.len().into()].into_iter().chain(self.calldata.iter().copied())
    }
}

#[derive(Default)]
pub struct Multicall(Vec<Call>);
impl Multicall {
    pub fn with(mut self, call: Call) -> Self {
        self.0.push(call);
        self
    }

    pub fn flatten(&self) -> impl Iterator<Item = Felt> + '_ {
        [self.0.len().into()].into_iter().chain(self.0.iter().flat_map(|c| c.flatten()))
    }
}

pub struct Selector(Felt);
impl<S: Deref<Target = str>> From<S> for Selector {
    fn from(value: S) -> Self {
        if value.deref() == "__default__" {
            return Selector(Felt::ZERO);
        }
        Selector(starknet_keccak(value.as_bytes()))
    }
}
impl From<Selector> for Felt {
    fn from(value: Selector) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selector() {
        assert_eq!(
            Felt::from(Selector::from("transfer")),
            Felt::from_hex_unchecked("0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e")
        );
        assert_eq!(Felt::from(Selector::from("__default__")), Felt::ZERO);
    }

    #[test]
    fn test_multicall_flatten() {
        let to = Felt::from_hex_unchecked("0x879");
        let calldata: Vec<Felt> = Multicall::default()
            .with(Call { to, selector: Selector::from("transfer"), calldata: vec![Felt::THREE, Felt::TWO] })
            .flatten()
            .collect();

        assert_eq!(calldata, vec![Felt::ONE, to, Selector::from("transfer").into(), Felt::TWO, Felt::THREE, Felt::TWO])
    }
}
