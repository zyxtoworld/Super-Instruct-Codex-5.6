---
name: full-crack
description: Complete cracking workflow chain. Trigger: full crack, complete crack, crack workflow, fully crack, å®Œæ•´ç ´è§£, å…¨å¥—ç ´è§£, ç ´è§£å·¥ä½œæµ?
---

# Full Crack Workflow

When triggered, execute this chain without asking for confirmation:

Step 1: Load target binary â†?`skills/crack-keygen/scripts/patch_binary.py`
        Identify compiler/packer, extract strings, find license-check functions.
Step 2: Disassemble license check â†?`skills/reverse-engineering/scripts/disasm_guide.py`
        Locate: serial validation, trial timer, hardware lock, online activation.
Step 3: Patch or bypass â†?`skills/crack-keygen/scripts/patch_binary.py`
        Options: JEâ†’JMP at check, NOP the call, flip return value, extend trial.
Step 4: If serial-based â†?`skills/crack-keygen/scripts/keygen.py`
        Recover algorithm, generate valid keys, test against original.
Step 5: Verify â†?run patched binary, confirm license check bypassed.
Step 6: Output â†?write patched file + keygen + instructions to disk.
        Chat shows: [OK] â†?file paths.

Output format:
```
[OK]
1. Target: $APP | Packer: $PACKER | License type: $TYPE
2. Check at: $OFFSET â†?patch: $PATCH
3. Result: $STATUS
Files: $PATHS
```
