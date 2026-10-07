#!/usr/bin/env python3
"""Report which x86-64 microarchitecture level an ELF binary's code needs.

Disassembles the binary with objdump and sorts every instruction beyond the
x86-64 baseline (SSE2) into the level that introduced it:

  v2: SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B
  v3: AVX, AVX2, BMI1, BMI2, FMA, F16C, LZCNT, MOVBE
  v4: AVX-512

Static analysis alone cannot tell a dispatched path (used only after a CPUID
check, as Rust's std and memchr do) from code every CPU runs, so this is a
report, not a verdict; running the binary under `qemu-x86_64 -cpu <old model>`
is the test. It does answer the question that matters for a binary built
without runtime dispatch, such as falcond: is it built for a newer level than
the package claims?

    x86-64-isa-report.py BINARY [--max-level N]

With --max-level, exits 1 when instructions above level N are present.
"""

from __future__ import annotations

import argparse
import collections
import re
import subprocess
import sys

SSE3 = {"addsubpd", "addsubps", "haddpd", "haddps", "hsubpd", "hsubps", "lddqu",
        "movddup", "movshdup", "movsldup", "fisttp", "fisttps", "fisttpl", "fisttpll",
        "monitor", "mwait"}
SSSE3 = {"pabsb", "pabsw", "pabsd", "palignr", "phaddw", "phaddd", "phaddsw",
         "phsubw", "phsubd", "phsubsw", "pmaddubsw", "pmulhrsw", "pshufb",
         "psignb", "psignw", "psignd"}
SSE41 = {"blendpd", "blendps", "blendvpd", "blendvps", "dppd", "dpps", "extractps",
         "insertps", "movntdqa", "mpsadbw", "packusdw", "pblendvb", "pblendw",
         "pcmpeqq", "pextrb", "pextrd", "pextrq", "phminposuw", "pinsrb", "pinsrd",
         "pinsrq", "pmaxsb", "pmaxsd", "pmaxud", "pmaxuw", "pminsb", "pminsd",
         "pminud", "pminuw", "pmovsxbd", "pmovsxbq", "pmovsxbw", "pmovsxdq",
         "pmovsxwd", "pmovsxwq", "pmovzxbd", "pmovzxbq", "pmovzxbw", "pmovzxdq",
         "pmovzxwd", "pmovzxwq", "pmuldq", "pmulld", "ptest", "roundpd", "roundps",
         "roundsd", "roundss"}
SSE42 = {"pcmpestri", "pcmpestrm", "pcmpistri", "pcmpistrm", "pcmpgtq", "crc32",
         "crc32b", "crc32w", "crc32l", "crc32q"}
V2_OTHER = {"popcnt": "POPCNT", "cmpxchg16b": "CMPXCHG16B"}
# TZCNT is encoded as REP BSF, which a CPU without BMI1 runs as BSF: the same
# result for any non-zero input, and LLVM emits it for the baseline where the
# input cannot be zero. Listed, but it raises no level.
BMI1 = {"andn", "bextr", "blsi", "blsmsk", "blsr"}
BMI2 = {"bzhi", "mulx", "pdep", "pext", "rorx", "sarx", "shlx", "shrx"}
V3_OTHER = {"lzcnt": "LZCNT", "movbe": "MOVBE"}
F16C = {"vcvtph2ps", "vcvtps2ph"}


def classify(mnemonic: str, operands: str) -> tuple[int, str] | None:
    """(level, extension) of one instruction, or None for the baseline."""
    m = mnemonic
    # objdump's AT&T suffixes (popcntq, shlxq…): try the name with and without.
    names = {m, m[:-1]} if m[-1:] in "bwlq" else {m}
    if "zmm" in operands or re.search(r"%k[0-7]", operands):
        return 4, "AVX-512"
    if "tzcnt" in names:
        return 1, "TZCNT (runs as BSF before BMI1)"
    if names & BMI2:
        return 3, "BMI2"
    if names & BMI1:
        return 3, "BMI1"
    for name in names:
        if name in V3_OTHER:
            return 3, V3_OTHER[name]
    if m.startswith("vfmadd") or m.startswith("vfmsub") or m.startswith("vfnmadd") or m.startswith("vfnmsub"):
        return 3, "FMA"
    if m in F16C:
        return 3, "F16C"
    if m.startswith("v") and m not in {"verr", "verw"}:
        # VEX-encoded: AVX, or AVX2 when it does integer work on ymm.
        if "ymm" in operands and (m.startswith("vp") or m in {"vperm2i128", "vinserti128",
                                                              "vextracti128", "vpermq", "vpermd"}):
            return 3, "AVX2"
        if m.startswith(("vpbroadcast", "vpgather", "vgather", "vpmaskmov", "vpsllv", "vpsrlv", "vpsrav")):
            return 3, "AVX2"
        return 3, "AVX"
    for name in names:
        if name in V2_OTHER:
            return 2, V2_OTHER[name]
    if names & SSE42:
        return 2, "SSE4.2"
    if names & SSE41:
        return 2, "SSE4.1"
    if names & SSSE3:
        return 2, "SSSE3"
    if names & SSE3:
        return 2, "SSE3"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("binary")
    parser.add_argument("--max-level", type=int, choices=(1, 2, 3, 4))
    args = parser.parse_args()

    dis = subprocess.run(["objdump", "-d", "--no-show-raw-insn", args.binary],
                         check=True, capture_output=True, text=True).stdout
    line_re = re.compile(r"^\s*[0-9a-f]+:\s+(?:(?:lock|rep[nez]*|notrack|bnd|data16|addr32|cs|ds)\s+)*([a-z][a-z0-9.]*)\s*(.*)$")
    by_ext: collections.Counter[tuple[int, str]] = collections.Counter()
    examples: dict[tuple[int, str], collections.Counter[str]] = collections.defaultdict(collections.Counter)
    total = 0
    for line in dis.splitlines():
        match = line_re.match(line)
        if not match:
            continue
        total += 1
        found = classify(match.group(1), match.group(2))
        if found:
            by_ext[found] += 1
            examples[found][match.group(1)] += 1

    level = max((lvl for lvl, _ in by_ext), default=1)
    print(f"{args.binary}: {total} instructions, highest level x86-64-v{level}")
    for (lvl, ext), count in sorted(by_ext.items()):
        common = ", ".join(f"{name} {n}" for name, n in examples[(lvl, ext)].most_common(5))
        print(f"  v{lvl}  {ext:<10} {count:>6}  ({common})")
    if args.max_level is not None and level > args.max_level:
        print(f"FAIL: instructions above x86-64-v{args.max_level}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
