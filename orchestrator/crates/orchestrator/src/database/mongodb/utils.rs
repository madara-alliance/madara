use color_eyre::eyre::{eyre, Result};
use mongodb::bson::{Bson, Document};
use serde::Serialize;

pub trait ToDocument {
    fn to_document(&self) -> Result<Document>;
}

impl<T: Serialize> ToDocument for T {
    fn to_document(&self) -> Result<Document> {
        match mongodb::bson::to_bson(self)? {
            Bson::Document(document) => Ok(document),
            _ => Err(eyre!("Failed to convert to Document")),
        }
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, Document};
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Serialize, Deserialize)]
    struct TestStruct {
        id: i32,
        name: String,
        optional_field: Option<String>,
    }

    #[test]
    fn test_to_document() {
        let test_struct = TestStruct { id: 1, name: "Test".to_string(), optional_field: None };

        let document = test_struct.to_document().expect("Failed to convert to Document");

        let mut expected_document = Document::new();
        expected_document.insert("id", 1);
        expected_document.insert("name", "Test");
        expected_document.insert("optional_field", Bson::Null);

        assert_eq!(document, expected_document);
    }

    #[test]
    fn test_to_document_fail() {
        let non_document_value = 1;

        let result = non_document_value.to_document();

        assert!(result.is_err() && result.unwrap_err().to_string().contains("Failed to convert to Document"));
    }
}
