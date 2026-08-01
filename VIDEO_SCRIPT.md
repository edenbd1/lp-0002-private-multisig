# 🎬 LP-0002 — script vidéo (~6 min)

> **⚠️ Tap les liens directement sur ton phone — ne pas copy-paste (les URLs sont longues et peuvent être tronquées au copy).**

- Durée totale visée : **~6 minutes**
- Langue : English
- `🎬 ACTION` = ce que tu fais à l'écran
- `💬 SAY` = ce que tu lis à voix haute

> **Toutes les commandes de ce script ont été exécutées et vérifiées.** Les sorties
> annoncées sont les vraies. Si quelque chose ne sort pas comme écrit, arrête et
> dis-le-moi plutôt que d'improviser.

---

# ⚙️ Pré-vol (~3 min de prep)

## Terminal

**🎬 ACTION** :

```bash
cd /Users/eden/data/ns.com/lp-0002
clear
```

Agrandis la police (⌘+ plusieurs fois — le texte doit être lisible en 1080p),
ferme Slack/Discord/notifications, fenêtre terminal en plein écran.

## Un seul onglet browser

**🎬 ACTION : ONGLET A (le repo)** — tap ce lien :

→ **[Repo : edenbd1/lp-0002-private-multisig](https://github.com/edenbd1/lp-0002-private-multisig)**

> **Pas d'onglet block explorer cette fois.** Une transaction privacy ne publie
> ni `program_id` ni `instruction_data`, donc l'indexeur de l'explorer n'a rien à
> afficher pour une approbation. C'est la propriété de confidentialité qui
> fonctionne, pas un bug — et c'est exactement ce qu'on montre à la scène 3, en
> lisant la chaîne directement. Plus fort qu'un explorer, et ça ne peut pas
> t'afficher une page vide en pleine caméra.

## Vérif de dernière seconde (30 s, avant d'enregistrer)

**🎬 ACTION** :

```bash
P=$(cat artifacts/testnet/proposal_id)
./scripts/verify-onchain.sh artifacts/testnet $P
```

Tu dois voir **cinq ✅** et `all accounts present and owned by the verifier`.
Si oui → QuickTime → Screen Recording → Démarre. Sinon → stop, préviens-moi.

---

# SCÈNE 1 — Intro (0:00 – 0:45)

**🎬 ACTION** : Terminal vide

**💬 SAY** :

> "Hi, I'm Eden. This is my submission for Logos Lambda Prize L-P zero-zero-zero-two — Private M-of-N Multisig."

**💬 SAY** :

> "A threshold multisig where members hold shielded accounts. When M members approve, the chain records that the threshold was met — and nothing about which members met it. Not to outside observers, and, the part I care most about, not to the other members either. The brief asks for exactly that: without revealing identity to on-chain observers *or other members*."

**💬 SAY** :

> "The membership proof is genuinely verified on chain, and everything is live on the public L-E-Z testnet. Let me show you."

---

# SCÈNE 2 — Le demo (0:45 – 3:00)

**🎬 ACTION** : Tape lentement :

```bash
./scripts/demo.sh
```

**🎬 ACTION** : Entrée. Ça prend environ **5 secondes**. Laisse défiler,
**puis scrolle doucement vers le haut** pendant que tu parles.

**💬 SAY** (pendant que ça tourne) :

> "Runs from a clean clone. No network needed for the first ten steps, no funded account, no local sequencer."

**🎬 ACTION** : Scrolle jusqu'en haut, sur `== 0. environment`

**💬 SAY** :

> "First line — R-I-S-C zero dev mode equals zero. Real proofs, no mock receipts. That's required by the brief, and it's the first thing I show."

**🎬 ACTION** : Scrolle sur `== 1.` et `== 2.`

**💬 SAY** :

> "Twenty-five adversarial tests on the circuit logic — non-members, borrowed Merkle paths, invented member sets, forged nullifiers. Then twenty-eight more against the *built binary*, run through the sequencer's own executor. Same executor, same input order, same thirty-two megabyte session limit the chain applies. A rejection you see there is the rejection the chain performs. Plus two that pin the verifier to the exact membership binary it chains to, so it can't be swapped."

**🎬 ACTION** : Scrolle sur `== 6.` puis `== 7.`

**💬 SAY** :

> "Step six: a member tries to approve twice and is refused. Step seven: someone tries to swap the action under the same proposal id, and that's refused too. That second one matters — if approvals were scoped to a proposal id alone, you could collect signatures for a harmless action and then execute a malicious one under the same id."

**🎬 ACTION** : Scrolle sur `== 9. what an observer sees` — **ralentis ici**

**💬 SAY** (le cœur de la soumission — prends ton temps) :

> "This is the whole point. On chain, this proposal is three addresses. Each one is a hash of a nullifier, and each nullifier is a hash of a member secret. Someone who knows all five members — including the other four members — cannot tell which three of them these came from."

**🎬 ACTION** : Scrolle sur `== 10. compute cost`

**💬 SAY** :

> "Measured compute cost: approve is three hundred thirty-six thousand cycles, one and a half percent of the public budget. Execute scales linearly in M."

---

# SCÈNE 3 — La chaîne, en direct (3:00 – 4:15)

**🎬 ACTION** : Scrolle sur `== 11. the Basecamp package`

**💬 SAY** :

> "Step eleven checks the Basecamp package. The dot-l-g-x is committed, and the script recomputes its manifest hashes from its own contents — so the package is verified, not just present."

**🎬 ACTION** : Scrolle tout en bas, sur `== 12. the live deployment`

**💬 SAY** :

> "And the last step reads the live deployment straight off the public testnet. Five accounts, all owned by the verifier program: the multisig, the proposal, two approval markers, and the execution marker."

**💬 SAY** :

> "Those two approval markers are the whole claim. Each exists only because the approve instruction ran and claimed it. Approve declares a chained call to a L-E-Z-native membership program, and on the privacy path L-E-Z's circuit composes that call with a real env-verify, whose receipt the sequencer checks against the pinned circuit I-D. So neither marker could exist without a membership proof having been verified on chain."

**🎬 ACTION** : Tape (pour montrer que ce n'est pas mon script qui raconte ce qu'il veut) :

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
 -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["9q31RPufMoRe6pXcxrcuwFEJQN2Wnr2qV4HhXnV8a42r"]}'
```

**💬 SAY** :

> "Don't take my script's word for it. That's the raw chain, one of the approval markers. Owned by the verifier program. Zero bytes of data — it names nobody."

---

# SCÈNE 4 — L'audit (4:15 – 5:20)

**🎬 ACTION** : Reste sur le terminal

**💬 SAY** :

> "One more thing, because I think it matters more than a feature list."

**💬 SAY** :

> "After the first version was deployed, I ran an adversarial audit — writing tests that try to *break* the program rather than confirm it works. It found a threshold bypass in my own code."

**💬 SAY** :

> "The proposal account was addressed by a reference that commits to the multisig I-D — but a program can't invert a hash, and nothing re-derived it. So anyone could create their own one-member multisig with a threshold of one, approve someone else's proposal while naming their own multisig, and execute it. A five-of-nine proposal would execute on one signature from an outsider."

**💬 SAY** :

> "Every individual check was doing its job. The gap was between them. The fix puts the multisig I-D into the proposal's address, so that pairing now resolves to an account nobody ever created. I redeployed, re-ran the whole lifecycle, and wrote the finding up in docs slash security dot M-D rather than quietly patching it. Two regression tests pin both halves of the attack."

**💬 SAY** :

> "The same pass found three documented error codes with no test behind them, and a verification script that reported accounts as missing when they were on chain. All fixed. Fifty-seven tests now."

---

# SCÈNE 5 — Closing (5:20 – 6:00)

**🎬 ACTION** : Passe sur l'ONGLET A (le repo)

**💬 SAY** :

> "To summarize. Two programs deployed on the public L-E-Z testnet, byte-identical to what's in the repository — you can verify that from the deployment transaction. A full two-of-three lifecycle on chain: create, propose, two approvals on the privacy path, execute. Fifty-seven tests, C-I green on Linux and macOS, a reproducible build, a packaged Basecamp module, an S-D-K, a SPEL I-D-L, and a demo script that runs from a clean clone."

**💬 SAY** :

> "Repository at github dot com slash eden-b-d-one slash l-p dash zero zero zero two dash private dash multisig. The threat model, including what is deliberately *not* hidden, is in docs slash security dot M-D. Thank you for reviewing."

**🎬 ACTION** : Attends 2 secondes en silence

**🎬 ACTION** : Stop l'enregistrement

---

# 📝 Post-recording

1. QuickTime → Export → 1080p mp4 → `lp-0002-submission.mp4`
2. YouTube Studio → tap **[studio.youtube.com](https://studio.youtube.com)** → Upload → **Unlisted**
3. **Title** : `LP-0002 Private M-of-N Multisig — Lambda Prize submission (edenbd1)`
4. **Description** :

   ```
   Submission demo for Logos Lambda Prize LP-0002 — Private M-of-N Multisig.

   A threshold multisig on the Logos Execution Zone where approvals are
   unlinkable to members — including to the other members. The membership
   proof is genuinely verified on chain via LEZ's privacy-preserving
   transaction path.

   Repo:
   https://github.com/edenbd1/lp-0002-private-multisig

   In that repo:
     docs/DEPLOYMENT.md  — every transaction hash, and how to re-verify each
     docs/security.md    — the threat model, and what is deliberately not hidden

   Public testnet:
   https://testnet.lez.logos.co
   ```

5. Reviens avec **l'URL YouTube + "OK submit la PR"** → je prépare la PR, tu la relis, puis je l'ouvre

---

# 🆘 Cheat sheet — prononciation

- **LEZ** = "L-E-Z" (épelle)
- **SPEL** = "spell"
- **PDA** = "P-D-A" (épelle)
- **RISC0** = "risk zero"
- **msk** = "M-S-K" (épelle)
- **env::verify** = "env verify"
- **SHA256** = "SHA two fifty-six"
- **M-of-N** = "M of N"
- **IDL** = "I-D-L" (épelle)
- **nullifier** = "NULL-ifier"
- **edenbd1** = "eden-B-D-one"

---

# ⏱️ Timing récap

| Scène | De | À | Durée |
|---|---|---|---|
| 1. Intro | 0:00 | 0:45 | 45s |
| 2. demo.sh | 0:45 | 3:00 | 2m15 |
| 3. La chaîne en direct | 3:00 | 4:15 | 1m15 |
| 4. L'audit | 4:15 | 5:20 | 1m05 |
| 5. Closing | 5:20 | 6:00 | 40s |
| **Total** | | | **~6 min** |

Entre 5 et 8 minutes c'est bon. Le brief demande une narration qui explique
l'architecture et les décisions — pas un screencast muet. La scène 4 est ce qui
te distingue : elle montre que tu audites ton propre travail.

**Tu peux y aller. 🎬**
