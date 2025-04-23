//! Class compilation for Cairo v0 classes is not supported. This only affects very few classes on mainnet, which is why we decided
//! to just hardcode them.
//! Mainnet is the only chain we know of that still has cairo v0 contracts, and their deployment is not supported anymore.

use starknet_types_core::felt::Felt;
use std::ops::RangeInclusive;

const fn felt(s: &str) -> Felt {
    Felt::from_hex_unchecked(s)
}

/// (block_n, real_class_hash, computed)
#[rustfmt::skip]
const CLASS_HASHES: [(u64, Felt, Felt); 42] = [
    (1469, felt("0x4c53698c9a42341e4123632e87b752d6ae470ddedeb8b0063eaa2deea387eeb"), felt("0x33824657c011faae7ad92b05888182f345e270c457f61bf1553c7a21b435ad0")),
    (1737, felt("0x157d87bdad1328cbf429826a83545f6ffb6505138983885a75997ee2c49e66b"), felt("0x1b859de6eb61535c0051e151f1177f0905d64f77e2d09adc90ea462bbd9be75")),
    (2244, felt("0x5829a410055a7da53295c05b7ce39f1b99c202d49f6194ca000d93d35adf491"), felt("0xb9531cb5fb9e79710a4514c3af84e2f2e352d3b93068f6a3fdc4548ac6ed2d")),
    (2249, felt("0x621b53649ddc3170f4c7db984252e9de2fd54c8080686aa9aaf5803194a9179"), felt("0x4b625f3ee59464ed825ebaa63e086d1fc72ebcc888bb929202bccb2a2ea764")),
    (2700, felt("0x7319e2f01b0947afd86c0bb0e95029551b32f6dc192c47b2e8b08415eebbc25"), felt("0x530490b3bdad7dc5c219c8073edfe9f0cfb3fc19aec629ac8edfef03f1b3c4")),
    (2710, felt("0x6033d77f82f83d7be51e33669e9ba90d4007e2f8652e0a4c94ea42a2d528823"), felt("0x82bb7876b277a7f639020664f941be52e17bcff61383768c07f50f2667033a")),
    (2713, felt("0x590267e2a8bdb5a5c5c6b8f51751fc661866d96e6dec956f8562f54ecdffabc"), felt("0x4a6727a05420d67222ffb4baf70e9ffca8acaef4d87c73ed253e124547045b8")),
    (2809, felt("0x2759db6e3df9433b04c05e1dd1e634dd960fab8ea9b821bea4204e96ae68c9e"), felt("0x3f44071a1a7b7b93c328782f740a1c010385c801515d6e4fb3842b202309136")),
    (2889, felt("0x78389bb177405c8f4f45e7397e15f2a86f94a1fe911a5efff9d481de596b364"), felt("0x5c819d0010500bd2a190a1ac80b5d850014b966d9a2d272ec2dbe7fcd634ccc")),
    (2889, felt("0x2760f25d5a4fb2bdde5f561fd0b44a3dee78c28903577d37d669939d97036a0"), felt("0x43a47bc546d2fbfc8da80d7b2042b6cfaf789426d24fc8ee27c5d6e1d63b84e")),
    (2889, felt("0x52c7ba99c77fc38dd3346beea6c0753c3471f2e3135af5bb837d6c9523fff62"), felt("0x2fe57da108216a0baaa4cc96968e790a52b2114d67cbbd4e81fd404fdd7a560")),
    (2902, felt("0x305ee7a74fc480b0ec54bb306ab32d23da2b6c9b307423d23370eb5351220c3"), felt("0x797334fcd05213b807430ff3c35354aaa232b021e2ca6fedb5d77bb770ff693")),
    (2902, felt("0x673e3747b6f9cc2daf2c1f4766bd2ff809d7b6b2f09ac15ecaf33004716f188"), felt("0x5b18a5a1fa24c671d04c34165355eff1f409fae4530ad69022dc12bb87b7072")),
    (2902, felt("0x1d043d58f3eb68ba366fc2f68f828a2175ab5e58a1a8b2974f0555deb994eeb"), felt("0x18d3065f183d54be8d7d51d92237db2dd64c55eca309078d8ccc6a570ec91fe")),
    (2902, felt("0x4bf6dca9977880e1c8e7f77342f2149e332cca5ff12457c9addf7927793724"), felt("0x57dc648c02ef9b31dd3d4eb631b05f6127dbef32aabe68c2934bcb3660599aa")),
    (2902, felt("0x34e388dec948b45601538a1c73122501f0ef87cc9b461ff445c6aa2660ef9a9"), felt("0xe892c8f9a5103aac531d6366c7f2990ac0f59632c9890a19ed47a949364e52")),
    (2920, felt("0x222f474bc7de126c517bf7954cb421932f4fa16de4666971df5b1154ea0ed37"), felt("0xe04b7cb85eeb8f935adcbd4b69b224d49c3c0e1f5de0779a94370a4f88f8a6")),
    (2984, felt("0x3c5bce981e63371aefb15863d953bf81c7d987ee356f35be972b658e0949ea9"), felt("0x7265a10ea0a5f68221c41c43fa6ba8adda9758ebc3275d0fe70f205afad0d9c")),
    (2984, felt("0x36d42b84ce857f4aba65fdc6390fd7e3a529266025daae8066b4ab794931419"), felt("0x2622860fce7efed89d80f2c2ed7ace0e6bc4aec329ef99c5344dffa4527efcb")),
    (2984, felt("0x2b1b1566846a0ceb9d47b500ad5350b0cfa28ce854cf85967812b5753dbba68"), felt("0x581c6a7fa2563f0342b89498ee4254e1d6ea06ba34c4c9de0158fa7ed594f2e")),
    (2984, felt("0x1711dd3e05254c235c3667ab5ba87aa0fb0bf356113fd4ef18694438e8e351d"), felt("0x2c7584f5fcf7e9c3d0f260a0ba3df5125e8c45cc28784935f0e5e7fad8ea81b")),
    (3020, felt("0x26bbcef21762d7b856682610d5eb70ea57a5afd523bba190df71fc30366ae83"), felt("0x11ceafa16d9165fb270ee8bcbd0b531fbfed5194e32508525afd58b321f8ff2")),
    (3234, felt("0x69577e6756a99b584b5d1ce8e60650ae33b6e2b13541783458268f07da6b38a"), felt("0x72b1d5f9065d0934bc82691fd4baebddd4fe63560d2bdec0278f13ca33cfdf9")),
    (3274, felt("0x7b19114cf65bc824e91a6435f32b3445d28911030972750c4a8f7488c933615"), felt("0x13d36c2e3b5b5aa949eadd8ebc1ba03168436d2ab1e643ac3d3a8b6036979d9")),
    (3274, felt("0x702025b02d838976cf33dd7deec76b27a111d331b6093f2e7137e31c2f6ffd4"), felt("0x2131e6deb432f5944c042c574aeab82a5e16157ae663ac45f9f8b73ae1a321e")),
    (3311, felt("0xb9e54e5942404ec702f0a1fa436fdf29e88eb3d45ab1f93e0178a483617e4d"), felt("0x1bbaf80ee33aedfa59524b762279010959826d397892dbabbe62701f1d08abe")),
    (3311, felt("0x69ecab4645830a7c8be160e69d8014ab8c9c5bb30943438133aa6db8af3aaf2"), felt("0xb01e1a775da5f2b2387b7e1bdfa316b1a751d3c80c3e9057b6b24ec9e4a6d9")),
    (3799, felt("0x4cf9006236d8b234c9a3063e463e392a51293f45d9512f110636ad36042c449"), felt("0x6d20e7b8edfcc94d5ad92c11cd7b53e9ffffb440d496d207862396aaacb3319")),
    (3799, felt("0x1ca349f9721a2bf05012bb475b404313c497ca7d6d5f80c03e98ff31e9867f5"), felt("0x5f7daef8b95c42af6ff7b879fb328bdd50a85cbc3a4bdac23851e9630d3cc68")),
    (3806, felt("0x4e041f25cc9941b52067c2fdd55bbb69609a6498858bd1a7a2c33ea96ad32e8"), felt("0x25b7ef872da3759d74991081790b1208dfb9f9fbd31f9f2d76d29cadfd41f36")),
    (3806, felt("0x61e395994135df8ce430b2a8bbd2d6c825040d77031e647ac4962cf74c5c424"), felt("0x3805ed1595d395ef34226fb5c18820ef05cc502101d279f3a22e94ff6c01a42")),
    (3851, felt("0x2797c4997ea0ca14401188bf6cbd4d89f0e632bd088088ab6cabf63c2056fc8"), felt("0x822ce91c1de4ad315eedf205adfd531fa4793ade1a5bf493e8051ee5217381")),
    (3851, felt("0x7a9da578be6d793f0becd0001be1454e59622560e127dd983740c0c0619c23d"), felt("0x725809875c39e9caeff69f3a208c0e7a77a9712dab982be0e2702686c6ff346")),
    (3984, felt("0x6e0d9c78d2e51bd2c9017771fb038ddab1cd224ed5ad603bf96f06060714751"), felt("0x7a36cd7bc53cb87437f0f467aefff313ed1d8500d767fbcddb58c2f3778f419")),
    (4140, felt("0x65597f5a4c261c14f46767c5b269ec66294ca2d711cb6a4cd2cb694931f2b05"), felt("0x34d1fe27b454afc3196a9d61595f3a488093aab8d2092294510bfa1da22f9a2")),
    (4182, felt("0xa0cb53aaa37a4d377736e7e98c1a96b5168d75e3705f30fb09e6d2cbd7d5e3"), felt("0x2c11dee8e7ed84a47f178ea1fce823c22932e8b1762018105910edd55148e9e")),
    (4182, felt("0x2121cfb275789e8ed38777bec9636c217771bc8f6d27b2f1e9ef2f3ee155e9a"), felt("0x5fbc2f4a616c2f2ea613e3fc1dc0a3872d5064f2d9354ce7f807e9d6b3f4130")),
    (4182, felt("0x51b8d6defb074ef23ec0866a8b89f8224fa43d5a393de29f3bb6fa67e730c27"), felt("0x6cd352397a5108564c686af7b76fce78375f68f963a7f6bfcdf4fc0a38d21a2")),
    (4320, felt("0x28964626a3286d1814bdaeb309e7983a6549436d872f2d252c1dddbeb649c2e"), felt("0x1537cf2523d29c4ec5fb41b7b922b1cdc2e3da6c6109c2979a86dc787a137af")),
    (4390, felt("0x2a23536dffe8ccd28e7226cbae380e0383e8a286c15a5f7cb6363fd4cd46b43"), felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7")),
    (19097, felt("0x6dc10e7703c1b63e0b5a4e8e7842293d3255fd4e53d4e730adf435c3dffabb"), felt("0x117ab3f7b9e740cb01fb928514b2195888e094846cc3a1d534b8e8081332fbf")),
    (20732, felt("0x371b5f7c5517d84205365a87f02dcef230efa7b4dd91a9e4ba7e04c5b69d69b"), felt("0x92d5e5e82d6eaaef47a8ba076f0ea0989d2c5aeb84d74d8ade33fe773cbf67")),
];

const BLOCK_N_RANGE: RangeInclusive<u64> = CLASS_HASHES[0].0..=CLASS_HASHES[CLASS_HASHES.len() - 1].0;

pub fn get_real_class_hash(block_n: u64, computed: Felt) -> Felt {
    if !BLOCK_N_RANGE.contains(&block_n) {
        return computed;
    }

    // binary search
    // the partition point is the first index in the array where n is >= to block_n
    let partition_point = CLASS_HASHES.partition_point(|(n, _, _)| n < &block_n);

    for (entry_block_n, entry_real_hash, entry_computed) in &CLASS_HASHES[partition_point..] {
        if entry_block_n != &block_n {
            // we overshot the block_n
            break;
        }

        if entry_computed == &computed {
            // found :) return our override hash
            return *entry_real_hash;
        }
    }

    // not found
    computed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_override_hash() {
        assert_eq!(get_real_class_hash(0, Felt::ONE), Felt::ONE);
        assert_eq!(
            get_real_class_hash(2700, felt("0x530490b3bdad7dc5c219c8073edfe9f0cfb3fc19aec629ac8edfef03f1b3c4")),
            felt("0x7319e2f01b0947afd86c0bb0e95029551b32f6dc192c47b2e8b08415eebbc25"),
            "should find"
        );
        assert_eq!(get_real_class_hash(2700, felt("0x530490b3dedededed")), felt("0x530490b3dedededed"), "wrong hash");
        assert_eq!(
            get_real_class_hash(2701, felt("0x530490b3bdad7dc5c219c8073edfe9f0cfb3fc19aec629ac8edfef03f1b3c4")),
            felt("0x530490b3bdad7dc5c219c8073edfe9f0cfb3fc19aec629ac8edfef03f1b3c4"),
            "wrong block_n"
        );
        assert_eq!(
            get_real_class_hash(4390, felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7")),
            felt("0x2a23536dffe8ccd28e7226cbae380e0383e8a286c15a5f7cb6363fd4cd46b43"),
            "should find"
        );
        assert_eq!(
            get_real_class_hash(4389, felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7")),
            felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7"),
            "wrong block_n"
        );
        assert_eq!(
            get_real_class_hash(4391, felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7")),
            felt("0x68a63951ffbd6b4ff1840f00005e7a09a256d4b4ef3d3909cd475495bf370f7"),
            "wrong block_n"
        );
        assert_eq!(
            get_real_class_hash(2809, felt("0x4a6727a05420d67222ffb4baf70e9ffca8acaef4d87c73ed253e124547045b8")),
            felt("0x4a6727a05420d67222ffb4baf70e9ffca8acaef4d87c73ed253e124547045b8"),
            "wrong block_n"
        );
        assert_eq!(
            get_real_class_hash(2809, felt("0x3f44071a1a7b7b93c328782f740a1c010385c801515d6e4fb3842b202309136")),
            felt("0x2759db6e3df9433b04c05e1dd1e634dd960fab8ea9b821bea4204e96ae68c9e"),
            "should find"
        );
        assert_eq!(
            get_real_class_hash(2889, felt("0x5c819d0010500bd2a190a1ac80b5d850014b966d9a2d272ec2dbe7fcd634ccc")),
            felt("0x78389bb177405c8f4f45e7397e15f2a86f94a1fe911a5efff9d481de596b364"),
            "should find"
        );
        assert_eq!(
            get_real_class_hash(2889, felt("0x43a47bc546d2fbfc8da80d7b2042b6cfaf789426d24fc8ee27c5d6e1d63b84e")),
            felt("0x2760f25d5a4fb2bdde5f561fd0b44a3dee78c28903577d37d669939d97036a0"),
            "should find"
        );
        assert_eq!(
            get_real_class_hash(2889, felt("0x2fe57da108216a0baaa4cc96968e790a52b2114d67cbbd4e81fd404fdd7a560")),
            felt("0x52c7ba99c77fc38dd3346beea6c0753c3471f2e3135af5bb837d6c9523fff62"),
            "should find"
        );
        assert_eq!(
            get_real_class_hash(2889, felt("0x797334fcd05213b807430ff3c35354aaa232b021e2ca6fedb5d77bb770ff693")),
            felt("0x797334fcd05213b807430ff3c35354aaa232b021e2ca6fedb5d77bb770ff693"),
            "wrong block_n"
        );
    }
}
