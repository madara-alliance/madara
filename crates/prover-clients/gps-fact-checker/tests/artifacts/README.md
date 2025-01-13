# How to generate artifacts

## Solidity output for GPS verifier

Clone <https://github.com/starkware-libs/starkex-contracts> and build the repo
using Foundry.

## Cairo PIEs

In order to generate zip compressed Cairo PIEs follow the instructions at
<https://github.com/lambdaclass/cairo-vm/blob/main/cairo1-run/README.md>

Few things to note:

- Use `--cairo_pie_output` flag to specify the output path for the zipped PIE
  file
- Use `--append_return_values` flag to write program output to the related
  builtin segment
- Use the according layout (that includes `output` builtin at the very least,
  so by default `small`) depending on what particular program uses

Example:

```sh
cargo run ../cairo_programs/cairo-1-programs/fibonacci.cairo --append_return_values --cairo_pie_output fibonacci.zip --layout small
```

### Generate facts

To create test vectors for SHARP facts you would need to install the Cairo0
toolchain as described here: <https://docs.cairo-lang.org/quickstart.html#installation>

Then use the `get_fact.py` script to get the fact of the according zipped PIE.

Example:

```sh
python3 get_fact.py fibonacci.zip
```
