# A superseded deployment

These files record the LP-0002 lifecycle that ran on the public LEZ testnet
against verifier ImageID
`5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82` — a 2-of-3
multisig created, proposed, approved to threshold on the privacy-preserving
path, and executed. The seven transactions are still on chain and still valid.

**They are not this repository's program.** Giving the accounts state and the
threshold a treasury changed the verifier guest, and on LEZ a program's identity
is its ImageID, so the five account addresses here belong to a program the
repository no longer contains. `docs/DEPLOYMENT.md` has the full account.

Two consequences, stated rather than left to be discovered:

* **`msig` cannot read `proposals/<id>.json`.** It predates the typed action, so
  it carries an `action` string where the current schema carries `recipient`,
  `amount` and `memo_hash`. A `status --dir artifacts/testnet` fails to parse it,
  which is correct: the values in it were derived by a different action encoding
  and migrating them in place would produce a file whose `action_hash` did not
  match its own fields.
* **`scripts/verify-onchain.sh` against this directory reports every account
  absent**, because it derives addresses from the committed binary and that is a
  different program now.

What still checks out is `./scripts/verify-onchain-lifecycle.sh`, which reads the
transaction hashes directly. It is kept, not deleted, because a link that quietly
disappears is worse than one labelled.
