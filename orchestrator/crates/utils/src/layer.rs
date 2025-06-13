use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Layer {
    L2,
    L3,
}
impl FromStr for Layer {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "l2" => Ok(Layer::L2),
            "l3" => Ok(Layer::L3),
            "Layer2" => Ok(Layer::L2),
            "Layer3" => Ok(Layer::L3),
            other => Err(format!("Unknown layer: {}", other)),
        }
    }
}
