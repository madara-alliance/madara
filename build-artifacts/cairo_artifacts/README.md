# Cairo artifacts

We commit these artifacts so that the compilation process is easier, as they require different cairo 0 / cairo 1 versions.

The OZ contracts were compiled as to match the [`starkli`'s allowed class hash list](https://github.com/xJonathanLEI/starkli/blob/1bed33383d8f8cec926f7ce97c4a4243b8bbe43b/src/account.rs#L109):
OpenZeppelin v0.13.0 compiled with cairo v2.6.3.

```sh
git clone https://github.com/OpenZeppelin/cairo-contracts
cd cairo-contracts

SCARB_VERSION=2.6.3
OZ_VERSION=0.13.0

git checkout v$OZ_VERSION
asdf install scarb $SCARB_VERSION
asdf shell scarb $SCARB_VERSION
scarb build
starkli class-hash target/dev/openzeppelin_AccountUpgradeable.contract_class.json
```

should always return class hash `0x00e2eb8f5672af4e6a4e8a8f1b44989685e668489b0a25437733756c5a34a1d6`, and
`target/dev/openzeppelin_AccountUpgradeable.contract_class.json` should match the file in this
folder.

`openzeppelin_ERC20Upgradeable.contract_class.json` is compiled using the same scarb and OZ versions.