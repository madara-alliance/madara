//! RevertError formatting helpers.
//!
//! Receipt commitment uses the canonical blockifier revert string. Any local filtering or
//! reshaping of the stack trace changes the revert-reason hash and therefore the receipt
//! commitment.

use blockifier::execution::stack_trace::{EntryPointErrorFrame, ErrorStack, ErrorStackSegment, PreambleType};
use blockifier::transaction::objects::RevertError;

/// Thin helpers which keep receipt formatting aligned with blockifier's canonical `Display`
/// implementation.
pub trait RevertErrorExt {
    /// Returns the receipt-facing revert error.
    ///
    /// Most errors are left unchanged so receipt commitments use blockifier's canonical
    /// formatting. The only exception is the nested constructor failure shape where
    /// Pathfinder strips VM traceback frames from the receipt string.
    fn format_for_receipt(self) -> RevertError;

    /// Formats the receipt-facing revert error without consuming the original value.
    fn format_for_receipt_string(&self) -> String;
}

impl RevertErrorExt for RevertError {
    fn format_for_receipt(self) -> RevertError {
        match self {
            RevertError::Execution(error_stack) if should_strip_vm_tracebacks_for_receipt(&error_stack) => {
                RevertError::Execution(strip_vm_tracebacks(error_stack))
            }
            other => other,
        }
    }

    fn format_for_receipt_string(&self) -> String {
        match self {
            RevertError::Execution(error_stack) if should_strip_vm_tracebacks_for_receipt(error_stack) => {
                strip_vm_tracebacks_ref(error_stack).to_string()
            }
            _ => self.to_string(),
        }
    }
}

fn should_strip_vm_tracebacks_for_receipt(error_stack: &ErrorStack) -> bool {
    // Pathfinder strips VM tracebacks for constructor failures wrapped in another
    // "Execution failed" frame. Limit the special case to that exact shape so normal
    // blockifier revert strings remain canonical.
    let has_constructor_frame = error_stack
        .stack
        .iter()
        .any(|segment| matches!(segment, ErrorStackSegment::EntryPoint(entry_point) if entry_point.preamble_type == PreambleType::Constructor));
    let has_nested_constructor_failure = error_stack
        .stack
        .iter()
        .any(|segment| matches!(segment, ErrorStackSegment::StringFrame(s) if s.starts_with("Execution failed. Failure reason:\n")));

    has_constructor_frame && has_nested_constructor_failure
}

/// Consuming variant used while building receipt values.
fn strip_vm_tracebacks(error_stack: ErrorStack) -> ErrorStack {
    ErrorStack {
        header: error_stack.header,
        stack: error_stack.stack.into_iter().filter(|segment| !matches!(segment, ErrorStackSegment::Vm(_))).collect(),
    }
}

/// Borrowing variant used when callers only need the formatted receipt string.
fn strip_vm_tracebacks_ref(error_stack: &ErrorStack) -> ErrorStack {
    let mut new_stack = ErrorStack { header: error_stack.header.clone(), stack: Vec::new() };

    for segment in &error_stack.stack {
        match segment {
            ErrorStackSegment::Vm(_) => {}
            ErrorStackSegment::EntryPoint(entry_point) => {
                new_stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
                    depth: entry_point.depth,
                    preamble_type: entry_point.preamble_type.clone(),
                    storage_address: entry_point.storage_address,
                    class_hash: entry_point.class_hash,
                    selector: entry_point.selector,
                }));
            }
            ErrorStackSegment::Cairo1RevertSummary(summary) => {
                new_stack.push(ErrorStackSegment::Cairo1RevertSummary(summary.clone()));
            }
            ErrorStackSegment::StringFrame(s) => {
                new_stack.push(ErrorStackSegment::StringFrame(s.clone()));
            }
        }
    }

    new_stack
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::stack_trace::{ErrorStackHeader, ErrorStackSegment, VmExceptionFrame};
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

    const EXPECTED_PATHFINDER_RECEIPT_8138315: &str = r#"Transaction execution has failed:
0: Error in the called contract (contract address: 0x0578a41eafe7e6f5a34ff42444ca7df1b04516fc2e6d4d9a65e329eeb75109de, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):
1: Error in the called contract (contract address: 0x02ceed65a4bd731034c01113685c831b01c15d7d432f71afb1cf1634b53a2125, class hash: 0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8, selector: 0x02730079d734ee55315f4f141eaed376bddd8c2133523d223a344c5604e0f7f8):
2: Error in the contract class constructor (contract address: 0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194):
Execution failed. Failure reason:
Error in contract (contract address: 0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194):
0x4661696c656420746f20646573657269616c697a6520706172616d202331 ('Failed to deserialize param #1').
"#;

    const RAW_TRACE_8138315: &str = r#"Transaction execution has failed:
0: Error in the called contract (contract address: 0x0578a41eafe7e6f5a34ff42444ca7df1b04516fc2e6d4d9a65e329eeb75109de, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):
Error at pc=0:2929:
Cairo traceback (most recent call last):
Unknown location (pc=0:56)
Unknown location (pc=0:1187)
Unknown location (pc=0:1670)
Unknown location (pc=0:2289)

1: Error in the called contract (contract address: 0x02ceed65a4bd731034c01113685c831b01c15d7d432f71afb1cf1634b53a2125, class hash: 0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8, selector: 0x02730079d734ee55315f4f141eaed376bddd8c2133523d223a344c5604e0f7f8):
Error at pc=0:774:
Cairo traceback (most recent call last):
Unknown location (pc=0:152)

2: Error in the contract class constructor (contract address: 0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194):
Execution failed. Failure reason:
Error in contract (contract address: 0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3, class hash: 0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: 0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194):
0x4661696c656420746f20646573657269616c697a6520706172616d202331 ('Failed to deserialize param #1').
"#;

    const EXPECTED_PATHFINDER_RECEIPT_8168453: &str = r#"Transaction execution has failed:
0: Error in the called contract (contract address: 0x03666fcf7f5c9195d08464c5f2713d756864220f02342bd2382f781afc1c2b0d, class hash: 0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):
Error at pc=0:7331:
Cairo traceback (most recent call last):
Unknown location (pc=0:188)
Unknown location (pc=0:2616)
Unknown location (pc=0:3553)
Unknown location (pc=0:4820)
Unknown location (pc=0:5564)
Unknown location (pc=0:6675)

1: Error in the called contract (contract address: 0x076f0c5e5a7c9ded2d875321902d958dafa28a40bd56b51b6c983df94d7e03c9, class hash: 0x0637eda47d4e51b44a71ae559a69601ea8fcda38dfc1345665a8465ebe02a2e9, selector: 0x00161dc77f8e29b5e4194910df4cf7368b6c3c4ef7168245d7b194c9402b3fa6):
Error at pc=0:651:
Cairo traceback (most recent call last):
Unknown location (pc=0:70)

2: Error in the contract class constructor (contract address: 0x03673e9f0d6396cac1c232bfbfb2155d7bcc3af7e0268e440b4822c8145e47c4, class hash: 0x07efbb7f0a20d7fa7d25ff24fff9a974695c109ff17696aa8a68b105542c5cd3, selector: UNKNOWN):
Deployment failed: contract already deployed at address 0x03673e9f0d6396cac1c232bfbfb2155d7bcc3af7e0268e440b4822c8145e47c4
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

    fn sepolia_8138315_revert_error() -> RevertError {
        let mut stack = ErrorStack { header: ErrorStackHeader::Execution, stack: Vec::new() };
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 0,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x0578a41eafe7e6f5a34ff42444ca7df1b04516fc2e6d4d9a65e329eeb75109de"),
            class_hash: class_hash!("0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c"),
            selector: Some(EntryPointSelector(felt!(
                "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 2929)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:56)\nUnknown location \
                 (pc=0:1187)\nUnknown location (pc=0:1670)\nUnknown location (pc=0:2289)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 1,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x02ceed65a4bd731034c01113685c831b01c15d7d432f71afb1cf1634b53a2125"),
            class_hash: class_hash!("0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8"),
            selector: Some(EntryPointSelector(felt!(
                "0x02730079d734ee55315f4f141eaed376bddd8c2133523d223a344c5604e0f7f8"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 774)),
            error_attr_value: None,
            traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:152)\n".to_string()),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 2,
            preamble_type: PreambleType::Constructor,
            storage_address: contract_address!("0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3"),
            class_hash: class_hash!("0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c"),
            selector: Some(EntryPointSelector(felt!(
                "0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194"
            ))),
        }));
        stack.push(ErrorStackSegment::StringFrame(
            "Execution failed. Failure reason:\nError in contract (contract address: \
             0x062d39dd09d4799967ad7201a2a7651ae7c9ace4722182b900329e0817aef9a3, class hash: \
             0x0012276b8ff0f4c1f5c3a087ddc53a263fda97a5ee784f66bcda65467be5a98c, selector: \
             0x028ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194):\n\
             0x4661696c656420746f20646573657269616c697a6520706172616d202331 ('Failed to deserialize \
             param #1').\n"
                .replace("             ", ""),
        ));
        RevertError::Execution(stack)
    }

    fn sepolia_8168453_revert_error() -> RevertError {
        let mut stack = ErrorStack { header: ErrorStackHeader::Execution, stack: Vec::new() };
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 0,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x03666fcf7f5c9195d08464c5f2713d756864220f02342bd2382f781afc1c2b0d"),
            class_hash: class_hash!("0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564"),
            selector: Some(EntryPointSelector(felt!(
                "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 7331)),
            error_attr_value: None,
            traceback: Some(
                "Cairo traceback (most recent call last):\nUnknown location (pc=0:188)\nUnknown location \
                 (pc=0:2616)\nUnknown location (pc=0:3553)\nUnknown location (pc=0:4820)\nUnknown \
                 location (pc=0:5564)\nUnknown location (pc=0:6675)\n"
                    .to_string(),
            ),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 1,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0x076f0c5e5a7c9ded2d875321902d958dafa28a40bd56b51b6c983df94d7e03c9"),
            class_hash: class_hash!("0x0637eda47d4e51b44a71ae559a69601ea8fcda38dfc1345665a8465ebe02a2e9"),
            selector: Some(EntryPointSelector(felt!(
                "0x00161dc77f8e29b5e4194910df4cf7368b6c3c4ef7168245d7b194c9402b3fa6"
            ))),
        }));
        stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
            pc: Relocatable::from((0, 651)),
            error_attr_value: None,
            traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:70)\n".to_string()),
        }));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 2,
            preamble_type: PreambleType::Constructor,
            storage_address: contract_address!("0x03673e9f0d6396cac1c232bfbfb2155d7bcc3af7e0268e440b4822c8145e47c4"),
            class_hash: class_hash!("0x07efbb7f0a20d7fa7d25ff24fff9a974695c109ff17696aa8a68b105542c5cd3"),
            selector: None,
        }));
        stack.push(ErrorStackSegment::StringFrame(
            "Deployment failed: contract already deployed at address \
             0x03673e9f0d6396cac1c232bfbfb2155d7bcc3af7e0268e440b4822c8145e47c4\n"
                .replace("             ", ""),
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

    #[test]
    fn sepolia_8138315_receipt_string_matches_pathfinder_and_not_raw_trace() {
        let raw = sepolia_8138315_revert_error().to_string();
        let actual = sepolia_8138315_revert_error().format_for_receipt_string();

        assert_eq!(raw, RAW_TRACE_8138315);
        assert_eq!(actual, EXPECTED_PATHFINDER_RECEIPT_8138315);
        assert_ne!(actual, raw);
    }

    #[test]
    fn sepolia_8138315_receipt_value_strips_constructor_chain_vm_tracebacks() {
        let actual = sepolia_8138315_revert_error().format_for_receipt();

        assert_eq!(actual.to_string(), EXPECTED_PATHFINDER_RECEIPT_8138315);
    }

    #[test]
    fn sepolia_8168453_receipt_string_keeps_outer_vm_tracebacks() {
        let raw = sepolia_8168453_revert_error().to_string();
        let actual = sepolia_8168453_revert_error().format_for_receipt_string();

        assert_eq!(actual, EXPECTED_PATHFINDER_RECEIPT_8168453);
        assert_eq!(actual, raw);
    }

    #[test]
    fn sepolia_8168453_receipt_value_matches_pathfinder() {
        let actual = sepolia_8168453_revert_error().format_for_receipt();

        assert_eq!(actual.to_string(), EXPECTED_PATHFINDER_RECEIPT_8168453);
    }
}
