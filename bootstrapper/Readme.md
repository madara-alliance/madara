# Madara Bootstrapper

[![Coverage Status](https://coveralls.io/repos/github/madara-alliance/madara-bootstrapper/badge.svg?branch=main)](https://coveralls.io/github/madara-alliance/madara-bootstrapper?branch=main)

Madara Bootstrapper is a tool that helps to deploy the **Token Bridge** & **Eth Bridge** contract
between a madara/katana Appchain and another L2 or L1 network. It will also declare wallet
contracts from **OpenZappelin**, **Argent** and **Braavos**. You can find the full list of contracts
in the [Information](#information) section.

## Index

- [Madara Bootstrapper](#madara-bootstrapper)
  - [Index](#index)
  - [Testing](#testing)
    - [Important Notes](#important-notes)
  - [Run](#run)
    - [Local](#local)
    - [Docker](#docker)
  - [Information](#information)
    - [Contract Descriptions](#contract-descriptions)

**Currently Supported:**

- Madara App Chain <----> Ethereum / EVM based chains
- More coming soon...

## Testing

There are three test in the repository:

- bridge deployment e2e
- eth bridge deposit and claim
- erc20 token bridge deposit and claim

### Important Notes

- You need to comment/remove the #[ignore] tags in [crates/bootstrapper/src/tests/mod.rs](crates/bootstrapper/src/tests/mod.rs) file
- Only one test can be run at one time as all the tests are e2e tests.
- You also would need to restart both the chains after running each test.
- You would need to clone [Madara](https://github.com/madara-alliance/madara.git) repo by running :

  ```shell
  git clone --branch d188aa91efa78bcc54f92aa1035295fd50e068d2 https://github.com/madara-alliance/madara.git
  ```

```shell
# 1. Run madara instance with eth as settlement layer :
./target/release/madara --dev --da-layer=ethereum --da-conf=examples/da-confs/ethereum.json --settlement=Ethereum --settlement-conf=examples/da-confs/ethereum.json
# 2. Run anvil instance
~/.foundry/bin/anvil

# 3. Run tests
RUST_LOG=debug cargo test -- --nocapture
```

## Run

### Local

You can provide the env variables as arguments also, or you can also provide them in .env file.

Refer [.env.example](.env.example) file for setup

```shell
cp .env.example .env
cargo build --release
RUST_LOG=info cargo run -- --help

# If you have provided env vars in .env
RUST_LOG=info cargo run

# To run in dev mode (uses unsafe proxy and minimal setup)
RUST_LOG=info cargo run -- --dev
```

**IMP 🚨** : It will store all the addresses in [data/addresses.json](data/addresses.json)

### Docker

1. You need to set up the .env file first. Fill all the variables in .env file

   ```shell
   cp .env.example .env
   ```

2. You need to run docker compose command to build the image

   ```shell
   docker compose build
   ```

3. Run the image

   ```shell
   # If both the networks are running locally
   docker compose -f docker-compose-local.yml up
   # If you are hosting on already deployed networks
   docker compose up
   ```

**IMP 🚨** : It will store all the addresses in [data/addresses.json](data/addresses.json)

### Ubuntu Setup 🐧

To run the Madara Bootstrapper on an Ubuntu machine with AMD architecture, please follow these steps:

1. **Clone the Repository**: Start by cloning the Madara Bootstrapper repository.

   ```shell
   git clone https://github.com/madara-alliance/madara-bootstrapper.git
   cd madara-bootstrapper
   ```

2. **Install Build Essentials**: Ensure you have the necessary build tools.

   ```shell
   sudo apt update
   sudo apt install build-essential
   ```

3. **Install Docker**: Docker should be installed and running.

   ```shell
   sudo apt install docker.io
   sudo systemctl start docker
   sudo systemctl enable docker
   ```

4. **Install Python**: Python3 and its virtual environment package are required.

   ```shell
   sudo apt install python3 python3-venv
   ```

5. **Install Node.js and npm**: These are required for JavaScript dependencies.

   ```shell
   sudo apt install nodejs npm
   ```

6. **Install asdf**: Use asdf for managing runtime versions.

   ```shell
   git clone https://github.com/asdf-vm/asdf.git ~/.asdf --branch v0.14.1
   ```

7. **Install Rust**: Use rustup to install Rust.

   ```shell
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup show
   ```

8. **Create a Python Virtual Environment**: Set up a virtual environment for Python dependencies.

   ```shell
   python3 -m venv venv
   source venv/bin/activate
   ```

9. **Build Artifacts**: Use the `make` command to build the necessary artifacts within the Madara Bootstrapper repository.

   ```shell
   make artifacts-linux # For Linux
   make artifacts # For macOS
   ```

   > **Note**: In case you get an error related to permission, try running the command with `sudo`

10. **Build the Rust Binary**: Compile the Rust project.

    ```shell
    cargo build --release
    ```

11. **Run the Setup Command for L1**: Execute the following command to set up the L1 environment.

    ```shell
    RUST_LOG=debug cargo run --release -- --mode setup-l1 --config src/configs/devnet.json
    ```

    > **Note**: The default configuration file is located at `src/configs/devnet.json`. Please update it according to your requirements.
    > **Note**: Make sure to update the `devnet.json` file as per your needs.

12. **Update Configuration**: After running the `setup-l1` command, you will receive a response similar to the following:

    ```json
    {
      "starknet_contract_address": "STARKNET_CONTRACT_ADDRESS",
      "starknet_contract_implementation_address": "STARKNET_CONTRACT_IMPLEMENTATION_ADDRESS"
    }
    ```

    Update these values in your `devnet.json` or your specific configuration file before proceeding.

13. **Run the Setup Command for L2**: Now, execute the following command to set up the L2 environment.

    ```shell
    RUST_LOG=debug cargo run --release -- --mode setup-l2 --config src/configs/devnet.json
    ```

## Information

### Contract Descriptions

| Contract                                     | Source Link                  | Local Path               |
| -------------------------------------------- | ---------------------------- | ------------------------ |
| OpenZeppelinAccount (legacy: starknet)       | [Starkscan][oz-account]      | [JSON file][oz-json]     |
| OpenZeppelinAccount (modified: openzeppelin) | [account.cairo][account-src] | [Sierra file][oz-sierra] |
| UDC (Universal Deployer Contract)            | [Starkscan][udc-link]        | [JSON file][udc-json]    |

[oz-account]: https://sepolia.starkscan.co/class/0x05c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6
[oz-json]: ./src/contracts/OpenZeppelinAccount.json
[account-src]: src/contracts/account.cairo
[oz-sierra]: ./src/contracts/OpenZeppelinAccountCairoOne.sierra.json
[udc-link]: https://sepolia.starkscan.co/class/0x07b3e05f48f0c69e4a65ce5e076a66271a527aff2c34ce1083ec6e1526997a69
[udc-json]: ./src/contracts/udc.json

Here are some contract descriptions on why they are used in our context.

- `OpenZeppelinAccount (legacy: starknet)`: Contract used for declaring a temp account for declaring V1
  contract that will be used to deploy the user account with provided private key in env.
- `OpenZeppelinAccount (modified: openzeppelin)`: OZ account contract modified to include `deploy_contract`
  function as we deploy the UDC towards the end of the bootstrapper setup.

> [!IMPORTANT]
> For testing in Github CI we are using the madara binary build with `--disable-fee-flag`.
> The source for madara code: [madara-ci-build][madara-source]

[madara-source]: https://github.com/karnotxyz/madara/tree/madara-ci-build
