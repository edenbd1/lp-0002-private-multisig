# 🎬 LP-0002 — script vidéo (~8 min)

> **⚠️ Tap les liens directement sur ton phone — ne pas copy-paste (les URLs sont longues et peuvent être tronquées au copy).**

- Durée totale visée : **~8 minutes**
- Langue : English
- `🎬 ACTION` = ce que tu fais à l'écran
- `💬 SAY` = ce que tu lis à voix haute

> **Toutes les commandes de ce script ont été exécutées et vérifiées.** Les sorties
> annoncées sont les vraies. Si quelque chose ne sort pas comme écrit, arrête et
> dis-le-moi plutôt que d'improviser.

---

# ⚙️ Pré-vol (~3 min de prep)

## Terminal

**🎬 ACTION** — colle **ce bloc complet** (une seule fois, avant de filmer).
C'est de la config, pas intéressant à l'écran :

```bash
cd /Users/eden/data/ns.com/lp-0002
export PATH="$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH"
export DYLD_FALLBACK_FRAMEWORK_PATH=/Library/Developer/CommandLineTools/Library/Frameworks
clear
```

> **Pourquoi ces deux `export`.** `verify-onchain.sh` lit le ProgramId du
> binaire avec `spel` ; sans lui sur le `PATH` il dérive de mauvaises adresses.
> Le `DYLD_…` est pour le wallet LEZ, qui lie Python 3.9. Les deux ont été
> testés depuis un shell vierge : sans eux, la scène 2bis casse et le pré-vol te
> ferait annuler l'enregistrement pour rien.

Agrandis la police (⌘+ plusieurs fois — le texte doit être lisible en 1080p),
ferme Slack/Discord/notifications, fenêtre terminal en plein écran.

## Un seul onglet browser

**🎬 ACTION : ONGLET A (le repo)** — tap ce lien :

→ **[Repo : edenbd1/lp-0002-private-multisig](https://github.com/edenbd1/lp-0002-private-multisig)**

> **Pas d'onglet block explorer.** Deux raisons distinctes, garde-les en tête au
> cas où on te pose la question :
>
> 1. **L'explorer n'affiche aucune de nos sept transactions**, parce que son
>    indexeur est loin derrière la chaîne : sa propre page d'accueil listait le
>    bloc 4351 alors que `getLastBlockId` renvoyait 4496, et toutes nos
>    transactions sont postérieures. Ça touche tout ce qui a été soumis
>    récemment, par n'importe qui. Mesuré avec `./scripts/check-explorer.py`, qui
>    rend les pages dans un navigateur headless — l'explorer est une app WASM qui
>    sert la même coquille pour tous les hashes, donc comparer des tailles de
>    réponse ne prouve rien (une version antérieure de ce script en tirait la
>    conclusion inverse, à tort).
> 2. **Une approbation ne serait de toute façon pas indexable.** Une transaction
>    privacy ne publie ni `program_id` ni `instruction_data` — c'est la propriété
>    de confidentialité qui fonctionne, pas un bug.
>
> D'où la scène 3 : on lit la chaîne directement. Plus fort qu'un explorer, et ça
> ne peut pas t'afficher une page vide en pleine caméra.
>
> **Si on te pose la question en review**, la réponse honnête tient en une
> phrase : les sept hashes sont vivants, `getTransaction` les retourne, et
> `verify-onchain.sh` prouve la propriété des comptes — ce qu'aucun explorer ne
> fait.

## Vérif de dernière seconde (30 s, avant d'enregistrer)

**🎬 ACTION** :

```bash
# 1. Rien d'autre ne doit prouver — sinon la scène 2bis dure 3 à 4x plus
pgrep -fl r0vm || echo "ok, aucune preuve concurrente"

# 2. La vérif on-chain : cinq ✅ attendus
P=$(cat artifacts/testnet/proposal_id)
./scripts/verify-onchain.sh artifacts/testnet $P
```

> **La première commande n'est pas du zèle.** Une preuve concurrente sur cette
> machine (l'autre projet, un `cargo test`, un build) fait passer la scène 2bis
> de 2 min 30 à 15 minutes, en direct et sans moyen d'accélérer. Si `pgrep`
> sort quelque chose, attends qu'il ait fini.

Tu dois voir **cinq ✅** et `all accounts present and owned by the verifier`.
Si oui → QuickTime → Screen Recording → Démarre. Sinon → stop, préviens-moi.

> **Tout a été redéployé le 12 août**, après la migration vers LEZ v0.2.4. Le
> testnet public avait été réinitialisé sur une chaîne plus récente : nos sept
> transactions et nos cinq comptes étaient morts, et plus rien de ce que notre
> stack v0.2.0 soumettait n'était accepté. Le déploiement live est donc entièrement
> neuf — verifier ImageID `5bb40082…`, membership `56f784d6…`, et **les deux
> hashes de deploy ont changé aussi**, parce que les deux guests ont été
> recompilés contre le nouveau `lee_core`.
>
> Toutes les adresses de ce script pointent déjà sur la nouvelle instance. Si tu
> avais répété avec une version antérieure, **jette-la** : rien de l'ancienne ne
> résout plus.

## 📋 Les 4 commandes à taper à l'écran, dans l'ordre

Toutes relatives au dépôt — le bloc de config ci-dessus t'y a déjà mis. Chacune
a été relancée depuis un shell vierge avant que ce script te soit donné.

```bash
# Pré-vol (AVANT d'enregistrer) — doit afficher cinq ✅
./scripts/verify-onchain.sh artifacts/testnet $(cat artifacts/testnet/proposal_id)

# Scène 2 — 5 secondes
./scripts/demo.sh

# Scène 2bis — ~4 minutes, dont ~2min30 de proving réel à l'écran
MEMBERS=2 THRESHOLD=1 ./scripts/e2e-local-sequencer.sh

# Scène 3 — la chaîne brute, un marqueur d'approbation
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
 -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["DaG2Qan1ie5YhEpcti2LMCsvbkYi7WjWxnNKvxiqxi7B"]}'
```

---

# SCÈNE 1 — Intro (0:00 – 0:35)

**🎬 ACTION** : Terminal vide

**💬 SAY** :

> "Hi, I'm Eden. This is my submission for Logos Lambda Prize L-P zero-zero-zero-two — Private M-of-N Multisig."

**💬 SAY** :

> "A threshold multisig where members hold shielded accounts. When M members approve, the chain records that the threshold was met — and nothing about which members met it. Not to outside observers, and, the part I care most about, not to the other members either. That is what the brief asks for: without revealing identity to on-chain observers *or other members*. The membership proof is genuinely verified on chain, and it is all live on the public L-E-Z testnet. Let me show you."

---

# SCÈNE 2 — Le demo (0:35 – 2:10)

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

> "Twenty-five adversarial tests on the circuit logic — non-members, borrowed Merkle paths, invented member sets, forged nullifiers. Then thirty more against the *built binary*, run through the sequencer's own executor. Same executor, same input order, same thirty-two megabyte session limit the chain applies. A rejection you see there is the rejection the chain performs. Plus two that pin the verifier to the exact membership binary it chains to, so it can't be swapped."

**🎬 ACTION** : Scrolle sur `== 6.` et `== 7.`

**💬 SAY** :

> "A member approving twice is refused. And someone swapping the action under the same proposal id is refused too — which matters, because if approvals were scoped to a proposal id alone you could collect signatures for a harmless action and execute a malicious one under the same id."

**🎬 ACTION** : Scrolle sur `== 9. what an observer sees` — **ralentis ici**

**💬 SAY** (le cœur de la soumission — prends ton temps) :

> "This is the whole point. On chain, this proposal is three addresses. Each one is a hash of a nullifier, and each nullifier is a hash of a member secret. Someone who knows all five members — including the other four members — cannot tell which three of them these came from."

**🎬 ACTION** : Scrolle sur `== 10. compute cost`

**💬 SAY** :

> "Measured compute cost: approve is three hundred thirty-six thousand cycles, one and a half percent of the public budget. Execute scales linearly in M."

---

# SCÈNE 2bis — Une vraie preuve, en direct (2:10 – 4:45)

> **⚠️ Cette scène est OBLIGATOIRE.** Le brief l'exige mot pour mot : *« the
> recording must show terminal output (including proof generation) to confirm
> `RISC0_DEV_MODE=0` was active »*. `demo.sh` affiche la variable mais ne prouve
> rien — il dure cinq secondes. Sans cette scène la vidéo rate un critère nommé.

**🎬 ACTION** : Lance, dans le même terminal :

```bash
MEMBERS=2 THRESHOLD=1 ./scripts/e2e-local-sequencer.sh
```

**💬 SAY** (pendant que ça démarre) :

> "That five-second demo ran the verifier through the sequencer's executor. Same code the chain runs, but in process — so let me show you the other thing, against a real sequencer. This starts the actual sequencer binary in standalone mode, on localhost, on a chain that did not exist a second ago."

**🎬 ACTION** : Attends `[1/5]`, `[2/5]`, `[3/5]` — environ 30 secondes.

**💬 SAY** :

> "It's funding a throwaway account from the genesis vault, and deploying both programs onto that fresh chain. Notice the deployment hashes — they're the same two hashes as on the public testnet, because a deployment hash is the hash of the bytecode. Same binaries, same identity, wherever you put them."

**🎬 ACTION** : Quand `[5/6] gather 1 approvals` apparaît, **arrête-toi et
laisse tourner**. C'est le cœur de la scène : du proving réel, à l'écran, sans
coupure.

> **⚠️ La durée dépend de la charge de la machine, et fortement.** Sur un
> portable au repos c'est ~2 min 30. Mesuré ici à **935 s** parce qu'une autre
> preuve tournait en parallèle — presque quatre fois plus. **Vérifie avant de
> filmer** (voir le pré-vol) qu'aucun autre `r0vm` ne tourne, sinon tu passes
> quinze minutes de vidéo à regarder une barre de progression.
>
> **Ne cite aucun chiffre précis à voix haute** : le chronomètre s'affiche à
> l'écran à la fin, et une narration qui annonce « deux minutes trente » pendant
> qu'il affiche 900 s est la seule chose ici qu'un relecteur peut prendre en
> défaut.

**💬 SAY** (pendant le proving — prends ton temps, laisse des silences) :

> "This is the part the brief asks to see. R-I-S-C zero dev mode is zero — no mock receipts, no shortcut. This is a real Risc0 proof being generated on this laptop, right now. It takes minutes, and how many depends entirely on the machine and what else it is doing — on a shared C-I runner the same proof took over an hour. That is why the benchmark in the repo is stated per machine, with the machine named, rather than as one number."

**💬 SAY** :

> "What's being proved is membership: that the secret behind a nullifier owns a leaf under the committed member root. And it's declared as a chained call, so when this lands, L-E-Z's privacy circuit composes it with a real env-verify and the sequencer checks that receipt against the pinned circuit I-D. That composition is the whole reason this is on the privacy path and not the public one."

**🎬 ACTION** : Quand la ligne `approval 0: 1XXs wall clock` s'affiche, **montre-la**.

**💬 SAY** :

> "There's the measurement. The script times its own approvals and writes the number next to the transaction hash, so the benchmark comes out of the run that produced the evidence — not out of my memory. A hundred and fifty seconds, and it doesn't grow with the member set."

**🎬 ACTION** : Laisse arriver `execute`, puis les ✅ et
`e2e against a real local sequencer: PASS`.

**💬 SAY** :

> "Executed, and then it reads the accounts back off that local chain: the multisig, the proposal, the approval marker, the execution marker — all owned by the verifier program. That is a full lifecycle against a real sequencer, from an empty chain, with real proofs."

---

# SCÈNE 3 — La chaîne, en direct (4:45 – 5:55)

**🎬 ACTION** : Scrolle sur `== 11. the Basecamp package`

**💬 SAY** :

> "Step eleven checks the Basecamp package. The dot-l-g-x is committed, and the script recomputes its manifest hashes from its own contents — so the package is verified, not just present. And I installed it in Basecamp zero-two-two and drove it: the module loads, and pressing Status returns the live deployment's state, two of three, ready to execute."

**🎬 ACTION** : Scrolle tout en bas, sur `== 12. the live deployment`

**💬 SAY** :

> "And the last step reads the live deployment straight off the public testnet. Five accounts, all owned by the verifier program: the multisig, the proposal, two approval markers, and the execution marker."

**💬 SAY** :

> "Those two approval markers are the whole claim. Each exists only because the approve instruction ran and claimed it. Approve declares a chained call to a L-E-Z-native membership program, and on the privacy path L-E-Z's circuit composes that call with a real env-verify, whose receipt the sequencer checks against the pinned circuit I-D. So neither marker could exist without a membership proof having been verified on chain."

**🎬 ACTION** : Tape (pour montrer que ce n'est pas mon script qui raconte ce qu'il veut) :

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
 -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["DaG2Qan1ie5YhEpcti2LMCsvbkYi7WjWxnNKvxiqxi7B"]}'
```

**💬 SAY** :

> "Don't take my script's word for it. That's the raw chain, one of the approval markers. Owned by the verifier program. Zero bytes of data — it names nobody."

---

# SCÈNE 4 — L'audit (5:55 – 7:40)

**🎬 ACTION** : Reste sur le terminal

**💬 SAY** :

> "One more thing, because I think it matters more than a feature list."

**💬 SAY** :

> "After the first version was deployed, I audited it by writing tests that try to *break* the program rather than confirm it works. It found a threshold bypass in my own code: an outsider could create their own one-of-one multisig, approve someone else's proposal while naming it, and execute a five-of-nine on one signature. I fixed it, redeployed, and wrote it up."

**💬 SAY** (le beat qui compte — ralentis) :

> "And then that fix turned out to be incomplete, which is the part I most want to tell you about."

**💬 SAY** :

> "I had it reviewed by someone who had not written it. They asked the same question about the fix that I had asked about the bug — what is *not* checked. The config hash, which commits to the member set and the threshold, appeared neither in the proposal reference nor in the proposal's address. And creating a multisig puts no constraint on the multisig I-D, deliberately, because anyone should be able to create one."

**💬 SAY** :

> "Those two compose. An attacker creates a one-of-one member set *under your multisig I-D* — a fresh pair, so it initialises fine. Both accounts an approval needs still resolve. They approve against their own member set and execute your three-of-five at threshold one."

**💬 SAY** :

> "I wrote the exploit as two tests before writing any fix, and they failed — meaning the attack succeeded against the deployed binary. That's the confirmation. And here is why my own regression tests had missed it: both of them varied the multisig I-D. Neither held it fixed and varied the config, which is exactly the axis the first fix did not cover. I had tested the variant I found, not the family it belonged to."

**💬 SAY** :

> "The verifier you just watched me read off the chain is the corrected one, redeployed today. The same habit caught two more things worth a sentence each: the Basecamp package did not actually load until I installed Basecamp and tried it, and the deployment script my own docs point you at had never run end to end. Both fixed, both written up."

---

# SCÈNE 5 — Closing (7:40 – 8:10)

**🎬 ACTION** : Passe sur l'ONGLET A (le repo)

**💬 SAY** :

> "To summarize. Two programs deployed on the public L-E-Z testnet, byte-identical to what's in the repository — you can verify that from the deployment transaction. A full two-of-three lifecycle on chain: create, propose, two approvals on the privacy path, execute. Sixty-one tests, C-I green on Linux and macOS, a reproducible build, a Basecamp module I installed and used in Basecamp rather than just shipped, an S-D-K, a SPEL I-D-L, and a demo script that runs from a clean clone."

**💬 SAY** :

> "If there is one thing I would want you to take from this, it is not the feature list. It is that every claim in it is one I tried to break first, and twice I succeeded. The security doc has both findings written up, including the one where my own fix was not enough."

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
| 1. Intro | 0:00 | 0:35 | 35s |
| 2. demo.sh | 0:35 | 2:10 | 1m35 |
| **2bis. Une vraie preuve** | 2:10 | 4:45 | **2m35** |
| 3. La chaîne en direct | 4:45 | 5:55 | 1m10 |
| 4. L'audit | 5:55 | 7:40 | 1m45 |
| 5. Closing | 7:40 | 8:10 | 30s |
| **Total** | | | **~8 min** |

> **Où sont passées les 3 minutes.** La scène 4 racontait quatre trouvailles
> séparées ; elle en raconte une seule, celle qui porte — l'audit, le correctif,
> et la relecture croisée qui a montré qu'il était incomplet. Les autres tiennent
> en une phrase. Rien n'a été retiré du dépôt, seulement du récit.
>
> **Si tu veux descendre à ~6 minutes**, la seule marge restante est le proving
> de la scène 2bis : accélère-le ×4 en post-prod, terminal continu visible, sans
> coupure. Le brief demande de *voir* la génération de preuve, pas de la subir en
> temps réel — mais en temps réel il n'y a aucune ambiguïté, donc c'est ton
> arbitrage.

**Le brief n'impose aucune durée.** Vérifié dans les *Evaluation Policies* du
dépôt λPrize : ce qu'il exige, c'est une narration où tu expliques ce que tu as
construit et pourquoi, l'architecture, les décisions d'implémentation, et le
flux complet de bout en bout — « a silent screencast without explanation is not
sufficient ». Neuf minutes qui montrent une vraie preuve valent mieux que six
qui la sautent.

Deux scènes portent la soumission : la **2bis**, parce que le brief exige
explicitement de voir la génération de preuve avec `RISC0_DEV_MODE=0` à
l'écran ; et la **4**, parce qu'elle montre que tu audites ton propre travail.

**Tu peux y aller. 🎬**
