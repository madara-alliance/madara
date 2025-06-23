use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Layer {
    L2,
    L3,
}
impl FromStr for Layer {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "l2" => Ok(Layer::L2),
            "l3" => Ok(Layer::L3),
            "layer2" => Ok(Layer::L2),
            "layer3" => Ok(Layer::L3),
            other => Err(format!("Unknown layer: {}", other)),
        }
    }
}
