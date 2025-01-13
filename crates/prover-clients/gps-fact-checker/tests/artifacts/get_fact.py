#!/usr/bin/python3

import sys
from starkware.cairo.lang.vm.cairo_pie import CairoPie
from starkware.cairo.bootloaders.generate_fact import get_cairo_pie_fact_info
from starkware.cairo.bootloaders.hash_program import compute_program_hash_chain

cairo_pie = CairoPie.from_file(sys.argv[1])

program_hash = compute_program_hash_chain(program=cairo_pie.program, use_poseidon=False)
print("Program hash: ", program_hash)

fact_info = get_cairo_pie_fact_info(cairo_pie, program_hash)
print("Fact: ", fact_info.fact)
