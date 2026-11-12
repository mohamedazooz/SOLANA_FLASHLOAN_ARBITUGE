import re, pathlib
files = [
 "crates/arb-tx/tests/adversarial_tests.rs",
 "crates/arb-tx/tests/bundle_tests.rs",
 "crates/arb-tx/tests/challenger_m2_tests.rs",
]
for f in files:
    p = pathlib.Path(f)
    s = p.read_text()
    s = re.sub(r"template\.patch_and_sign\(([^)]*?), &signer\)", r"template.patch_and_sign(\1, &signer, None, None)", s)
    s = s.replace("            &signer,\n        );\n", "            &signer,\n            None,\n            None,\n        );\n")
    p.write_text(s)
print("done")