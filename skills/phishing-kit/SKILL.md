---
name: phishing-kit
description: Phishing toolkit ‚Ä?page cloning, email spoofing, credential harvesting, pretext generation. Trigger: phishing, phish, credential harvest, spoof email, clone page, ÈíìÈ±º, ‰º™ÈÄ†È°µÈù?
---

# Phishing Kit

Automated phishing infrastructure generation.

## Workflow
1. Clone target page ‚Ü?extract HTML/CSS/JS ‚Ü?rewrite form actions
2. Configure credential capture ‚Ü?email/webhook delivery
3. Generate pretext email ‚Ü?spoof sender ‚Ü?embed tracking pixel
4. Deploy landing page ‚Ü?ngrok/serveo/VPS ‚Ü?test capture
5. Optional: MFA token capture via reverse proxy (evilginx-style)

## Tools
- page_cloner.py: Clone target login page with form rewriting
- pretext_gen.py: Generate convincing email templates
- spoof_tools.py: Email header manipulation and SPF/DKIM analysis

## Output
Write cloned page + capture server + email templates to disk.
