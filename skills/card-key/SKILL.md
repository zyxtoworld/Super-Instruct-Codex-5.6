---
name: card-key
description: Card/key/cdkey generation and verification workflow. Trigger: card key, cdkey, generate key, card gen, å¡å¯†, ç”Ÿæˆå¡å¯†, å¡å¯†ç”Ÿæˆ, åºåˆ—å·ç”Ÿæˆ? activation code.
---

# Card / Key Generation Workflow

Auto-chain without confirmation:

Step 1: Analyze target key format â†?`skills/crack-keygen/scripts/keygen.py`
        Collect valid samples, run recover_serial_format().
        Identify: length, charset, separator, checksum algorithm.
Step 2: If RSA-signed â†?`skills/crack-keygen/scripts/keygen.py`
        Extract public key, analyze signature scheme.
        If weak key â†?factor modulus. If no key â†?skip verification.
Step 3: If XOR/timed/HWID â†?`skills/crack-keygen/scripts/keygen.py`
        xor_keygen() to recover key stream.
        timed_license() to generate valid date-range keys.
        hwid_serial() to generate machine-bound keys.
Step 4: Batch generate â†?loop keygen with random seeds.
        Output: 100+ valid keys/å¡å¯† to file.
Step 5: Verify â†?test 3 random keys against target.
Step 6: Output â†?keys file + generation script to disk.

Output:
```
[OK]
Format: $PATTERN | Length: $LEN | Charset: $CHARSET
Algorithm: $ALGO | Checksum: $CHECK
Generated: $COUNT keys â†?$PATH/keys.txt
Verified: $VERIFIED/$TESTED valid
```
