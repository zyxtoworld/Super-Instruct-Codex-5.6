---
name: vip-bypass
description: VIP/payment/subscription bypass workflow. Trigger: vip bypass, bypass vip, bypass payment, premium bypass, ç»•è¿‡VIP, ç»•è¿‡ä»˜è´¹, ç»•è¿‡ä¼šå‘˜, ä¼šå‘˜ç ´è§£, subscription bypass.
---

# VIP / Payment Bypass Workflow

Auto-chain without confirmation:

Step 1: Analyze target â†?identify payment/VIP check mechanism.
        Types: server-side verification, local flag, timer, ad-wall, feature gate.
Step 2: Local check â†?`skills/crack-keygen/scripts/patch_binary.py`
        Find: isVip(), isPremium(), isPro(), checkSubscription(), trialDays().
        Patch: return true, extend timer, disable check.
Step 3: Database/SQLite â†?`skills/crack-keygen/scripts/license_bypass.py`
        Find: user_level, vip_expire, premium_until, is_trial.
        Patch: set vip=1, expire=9999-12-31, trial=0.
Step 4: Android/iOS â†?`skills/crack-keygen/scripts/license_bypass.py`
        SharedPreferences, plist, Keychain â€?patch license flags.
Step 5: Server-side â†?proxy/mock the verification response.
        Hook HTTPS â†?return {"vip": true, "expire": "2099-12-31"}.
Step 6: Verify â†?launch app, confirm VIP features unlocked.
Step 7: Output â†?patch file + DB changes to disk.

Output:
```
[OK]
Target: $APP | VIP check: $TYPE | Method: $METHOD
Patch: $OFFSET â†?$BEFORE â†?$AFTER
Status: VIP UNLOCKED
Files: $PATHS
```
