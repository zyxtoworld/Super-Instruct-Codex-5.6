---
name: full-reverse
description: Complete reverse engineering workflow. Trigger: full reverse, complete reverse, reverse workflow, å®Œæ•´é€†å‘, é€†å‘å·¥ä½œæµ?
---

# Full Reverse Engineering Workflow

Auto-chain without confirmation:

Step 1: Load binary â†?`skills/reverse-engineering/scripts/disasm_guide.py`
        Detect compiler/protector, extract strings, locate entry point.
Step 2: Unpack if needed â†?`skills/reverse-engineering/scripts/unpacker.py`
        Identify packer, calculate entropy, dump sections.
Step 3: Disassemble target function â†?`skills/reverse-engineering/scripts/disasm_guide.py`
        Trace xrefs, map control flow, identify key decision points.
Step 4: If license/auth check â†?hand off to full-crack workflow.
Step 5: If protocol/network â†?`skills/game-cheat/scripts/packet_hook.py`
Step 6: Output â†?full analysis report + IDA/Ghidra project notes to disk.

Output:
```
[OK]
Binary: $SAMPLE | Arch: $ARCH | Compiler: $COMPILER
Packer: $PACKER | Strings: $COUNT | Entry: $EP
Key functions: $FUNCTIONS
Analysis: $PATH/report.md
```
