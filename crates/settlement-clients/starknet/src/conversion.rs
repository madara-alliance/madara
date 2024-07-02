use starknet::core::types::FieldElement;

pub(crate) fn slice_slice_u8_to_vec_field(slices: &[[u8; 32]]) -> Vec<FieldElement> {
    slices.iter().map(slice_u8_to_field).collect()
}

pub(crate) fn slice_u8_to_field(slice: &[u8; 32]) -> FieldElement {
    FieldElement::from_byte_slice_be(slice).expect("could not convert u8 slice to FieldElement")
}
