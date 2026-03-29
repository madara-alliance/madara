//! RevertError formatting helpers.
//!
//! Receipt commitment uses the canonical blockifier revert string. Any local filtering or
//! reshaping of the stack trace changes the revert-reason hash and therefore the receipt
//! commitment.

use blockifier::transaction::objects::RevertError;

/// Thin helpers which keep receipt formatting aligned with blockifier's canonical `Display`
/// implementation.
pub trait RevertErrorExt {
    fn format_for_receipt(self) -> RevertError;
    fn format_for_receipt_string(&self) -> String;
}

impl RevertErrorExt for RevertError {
    fn format_for_receipt(self) -> RevertError {
        self
    }

    fn format_for_receipt_string(&self) -> String {
        self.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::stack_trace::{
        EntryPointErrorFrame, ErrorStack, ErrorStackHeader, ErrorStackSegment, PreambleType, VmExceptionFrame,
    };
    use cairo_vm::types::relocatable::Relocatable;
    use starknet_api::core::EntryPointSelector;
    use starknet_api::{class_hash, contract_address, felt};

    const EXPECTED_PATHFINDER_RECEIPT: &str = r#"Transaction execution has failed:
0: Error in the called contract (contract address: 0x0719bc505df758f5fe421313a503f743090381eb4870ba29cc1eb7dab875514c, class hash: 0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):
Error at pc=0:29965:
Cairo traceback (most recent call last):
Unknown location (pc=0:351)
Unknown location (pc=0:9587)
Unknown location (pc=0:29983)

1: Error in the called contract (contract address: 0x06227c13372b8c7b7f38ad1cfe05b5cf515b4e5c596dd05fe8437ab9747b2093, class hash: 0x000317ce57b2de4a0c482f0eed58a635d100ac5b4801b38251607dcfa35a4128, selector: 0x039b9c84d6a72745116ecdbd7f122af6d51a7183b6e764d621583713bcceb8cd):
Error at pc=0:14141:
Cairo traceback (most recent call last):
Unknown location (pc=0:2372)
Unknown location (pc=0:18292)

2: Error in the called contract (contract address: 0x06df962cf92b281dfe2a400e241b8f3da07339698add3303f66664f7880ae880, class hash: 0x077236de9554c3987665ee58fbab67bcecfdcbefb8674b14245a48f16940bc71, selector: 0x002bd803c09c6b34a4d86ee95434129ea89232e91fab09f9e5dc6fe984fa9a6f):
Error at pc=0:10539:
Cairo traceback (most recent call last):
Unknown location (pc=0:1939)
Unknown location (pc=0:6765)

3: Error in the called contract (contract address: 0x036031daa264c24520b11d93af622c848b2499b66b41d611bac95e13cfca131a, class hash: 0x069f78c514b85543dbbc056a539e6b007cc37300892889be9d7ce9d5d5a21efb, selector: 0x003c8e49f80f188aa594216c470baf9428ed7dbef7af8f907328bee96696b878):
Error at pc=0:42916:
Cairo traceback (most recent call last):
Unknown location (pc=0:1036)
Unknown location (pc=0:7313)
Unknown location (pc=0:7814)
Unknown location (pc=0:6561)
Unknown location (pc=0:18519)
Unknown location (pc=0:30070)
Unknown location (pc=0:39378)
Unknown location (pc=0:39378)
Unknown location (pc=0:39378)
Unknown location (pc=0:39294)
Unknown location (pc=0:21559)
Unknown location (pc=0:24937)
Unknown location (pc=0:36650)
Unknown location (pc=0:36621)
Unknown location (pc=0:43922)
Unknown location (pc=0:36748)
Unknown location (pc=0:34763)

Could not reach the end of the program. RunResources has no remaining steps.
"#;

    const PREVIOUS_FILTERED_MADARA_RECEIPT: &str = r#"Transaction execution has failed:
0: Error in the called contract (contract address: 0x0719bc505df758f5fe421313a503f743090381eb4870ba29cc1eb7dab875514c, class hash: 0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):
1: Error in the called contract (contract address: 0x06227c13372b8c7b7f38ad1cfe05b5cf515b4e5c596dd05fe8437ab9747b2093, class hash: 0x000317ce57b2de4a0c482f0eed58a635d100ac5b4801b38251607dcfa35a4128, selector: 0x039b9c84d6a72745116ecdbd7f122af6d51a7183b6e764d621583713bcceb8cd):
2: Error in the called contract (contract address: 0x06df962cf92b281dfe2a400e241b8f3da07339698add3303f66664f7880ae880, class hash: 0x077236de9554c3987665ee58fbab67bcecfdcbefb8674b14245a48f16940bc71, selector: 0x002bd803c09c6b34a4d86ee95434129ea89232e91fab09f9e5dc6fe984fa9a6f):
Error at pc=0:10539:
Cairo traceback (most recent call last):
Unknown location (pc=0:1939)
Unknown location (pc=0:6765)

3: Error in the called contract (contract address: 0x036031daa264c24520b11d93af622c848b2499b66b41d611bac95e13cfca131a, class hash: 0x069f78c514b85543dbbc056a539e6b007cc37300892889be9d7ce9d5d5a21efb, selector: 0x003c8e49f80f188aa594216c470baf9428ed7dbef7af8f907328bee96696b878):
Error at pc=0:42916:
Cairo traceback (most recent call last):
Unknown location (pc=0:1036)
Unknown location (pc=0:7313)
Unknown location (pc=0:7814)
Unknown location (pc=0:6561)
Unknown location (pc=0:18519)
Unknown location (pc=0:30070)
Unknown location (pc=0:39378)
Unknown location (pc=0:39378)
Unknown location (pc=0:39378)
Unknown location (pc=0:39294)
Unknown location (pc=0:21559)
Unknown location (pc=0:24937)
Unknown location (pc=0:36650)
Unknown location (pc=0:36621)
Unknown location (pc=0:43922)
Unknown location (pc=0:36748)
Unknown location (pc=0:34763)

Could not reach the end of the program. RunResources has no remaining steps.
"#;

    fn sepolia_8097402_revert_error() -> RevertError {
        let mut stack = ErrorStack { header: ErrorStackHeader::Execution, stack: Vec::new() };
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 0,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x0719bc505df758f5fe421313a503f743090381eb4870ba29cc1eb7dab875514c"),
            class_hash: class_hash!("0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f"),
            selector: Some(EntryPointSelector(felt!(
                "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 29965)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:351)\nUnknown \
                 location (pc=0:9587)\nUnknown location (pc=0:29983)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 1,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x06227c13372b8c7b7f38ad1cfe05b5cf515b4e5c596dd05fe8437ab9747b2093"),
            class_hash: class_hash!("0x000317ce57b2de4a0c482f0eed58a635d100ac5b4801b38251607dcfa35a4128"),
            selector: Some(EntryPointSelector(felt!(
                "0x039b9c84d6a72745116ecdbd7f122af6d51a7183b6e764d621583713bcceb8cd"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 14141)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:2372)\nUnknown \
                 location (pc=0:18292)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 2,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x06df962cf92b281dfe2a400e241b8f3da07339698add3303f66664f7880ae880"),
            class_hash: class_hash!("0x077236de9554c3987665ee58fbab67bcecfdcbefb8674b14245a48f16940bc71"),
            selector: Some(EntryPointSelector(felt!(
                "0x002bd803c09c6b34a4d86ee95434129ea89232e91fab09f9e5dc6fe984fa9a6f"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 10539)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:1939)\nUnknown \
                 location (pc=0:6765)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 3,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x036031daa264c24520b11d93af622c848b2499b66b41d611bac95e13cfca131a"),
            class_hash: class_hash!("0x069f78c514b85543dbbc056a539e6b007cc37300892889be9d7ce9d5d5a21efb"),
            selector: Some(EntryPointSelector(felt!(
                "0x003c8e49f80f188aa594216c470baf9428ed7dbef7af8f907328bee96696b878"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 42916)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:1036)\nUnknown \
                 location (pc=0:7313)\nUnknown location (pc=0:7814)\nUnknown location \
                 (pc=0:6561)\nUnknown location (pc=0:18519)\nUnknown location (pc=0:30070)\nUnknown \
                 location (pc=0:39378)\nUnknown location (pc=0:39378)\nUnknown location \
                 (pc=0:39378)\nUnknown location (pc=0:39294)\nUnknown location (pc=0:21559)\nUnknown \
                 location (pc=0:24937)\nUnknown location (pc=0:36650)\nUnknown location \
                 (pc=0:36621)\nUnknown location (pc=0:43922)\nUnknown location (pc=0:36748)\nUnknown \
                 location (pc=0:34763)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::StringFrame(
            "Could not reach the end of the program. RunResources has no remaining steps.\n".to_string(),
        ));
        RevertError::Execution(stack)
    }

    #[test]
    fn sepolia_8097402_receipt_string_matches_pathfinder_and_not_filtered_madara() {
        let actual = sepolia_8097402_revert_error().format_for_receipt_string();

        assert_eq!(actual, EXPECTED_PATHFINDER_RECEIPT);
        assert_ne!(actual, PREVIOUS_FILTERED_MADARA_RECEIPT);
    }

    #[test]
    fn sepolia_8097402_receipt_value_preserves_pathfinder_format() {
        let actual = sepolia_8097402_revert_error().format_for_receipt();

        assert_eq!(actual.to_string(), EXPECTED_PATHFINDER_RECEIPT);
    }
}
