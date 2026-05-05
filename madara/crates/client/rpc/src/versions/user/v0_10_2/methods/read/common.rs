use mp_rpc::v0_10_2::ResponseFlag;

pub(super) fn response_flags_include_proof_facts(response_flags: Option<Vec<ResponseFlag>>) -> bool {
    response_flags.map(|flags| flags.contains(&ResponseFlag::IncludeProofFacts)).unwrap_or(false)
}
