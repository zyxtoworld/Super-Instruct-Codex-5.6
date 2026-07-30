---
name: vuln-scanner
description: Automated vulnerability scanning + exploitation workflow. Trigger: vuln scan, vulnerability scan, scan vuln, find vuln, æ¼æ´žæ‰«æ, æ‰«ææ¼æ´ž, CVE scan, vuln check.
---

# Vulnerability Scanner Workflow

Auto-chain without confirmation:

Phase 1 â€?Enumerate:
  `skills/network-pentest/scripts/port_scanner.py` â†?all open ports
  `skills/network-pentest/scripts/service_enum.py` â†?service versions

Phase 2 â€?CVE Match:
  Cross-reference service versions against known CVEs.
  Prioritize: RCE > privilege escalation > information disclosure.

Phase 3 â€?Exploit match:
  `skills/exploit-dev/scripts/shellcode_gen.py` â†?for matched CVEs
  `skills/exploit-dev/scripts/rop_chain.py` â†?for binary exploitation CVEs

Phase 4 â€?Validate:
  Test each CVE exploit. Record success/failure with evidence.

Phase 5 â€?Report:
  Write to disk: $HOST-vuln-report.md
  Format: CVE-ID | CVSS | Service | Exploit Status | Evidence

Chat shows: [OK] â†?$PATH/vuln-report.md
