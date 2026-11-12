import pathlib
p = pathlib.Path("crates/arb-ingest/src/lib.rs")
lines = p.read_text().splitlines(keepends=True)
# find start line index (1-based 719) and end before 767 (Build a quote...)
start = None
end = None
for i, l in enumerate(lines):
    if "Decode a Yellowstone account update" in l:
        # back up to the blank line + doc comment start; find preceding blank line
        start = i - 1  # blank line before doc comment? check
    if "Build a quote dependency set" in l:
        end = i
        break
print("start", start+1, "end", end+1)
# Remove from start to end-1 (keep the doc comment? no)
del lines[start:end]
p.write_text("".join(lines))
print("done")