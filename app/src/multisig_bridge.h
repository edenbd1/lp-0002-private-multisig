// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Bridge exposed to QML as `bridge`. It drives the multisig lifecycle by
// shelling out to the local `msig` CLI (crates/multisig-cli), which reuses the
// same multisig-core primitives the on-chain program verifies — so what the GUI
// computes and what the chain checks cannot drift apart.
//
// It holds no keys. A member points it at their multisig directory; the secret
// never leaves the CLI's process, and nothing here writes one to disk.

#pragma once

#include <QObject>
#include <QString>

class MultisigBridge : public QObject {
    Q_OBJECT
public:
    explicit MultisigBridge(QObject* parent = nullptr);

    // Create a member set and commit to (member_root, threshold).
    // Returns the human-readable summary, or an "error:"-prefixed string.
    Q_INVOKABLE QString newMultisig(const QString& dir, int members,
                                    int threshold, const QString& idHex);

    // Bind a treasury payment to a proposal id. Refuses to re-bind an id to a
    // different payment: the approvals already gathered would not carry over.
    // `amount` is a string rather than a number because it is a u128 on chain
    // and QML's numbers are doubles — a large amount would arrive rounded.
    Q_INVOKABLE QString propose(const QString& dir, const QString& proposalId,
                                const QString& recipientHex,
                                const QString& amount, const QString& memo);

    // Build one member's approval arguments. `memberIndex` for the demo set, or
    // pass a secret via `mskHex` for a member who holds only their own key.
    Q_INVOKABLE QString approve(const QString& dir, const QString& proposalId,
                                int memberIndex, const QString& mskHex,
                                const QString& outPath);

    // How many approvals have been gathered against the threshold. Reads the
    // resumable state file, so it survives a Basecamp restart.
    Q_INVOKABLE QString status(const QString& dir, const QString& proposalId);

    // Build the execution arguments once the threshold is reached.
    Q_INVOKABLE QString executeArgs(const QString& dir,
                                    const QString& proposalId,
                                    const QString& outPath);

    // Path to the `msig` binary, overridable from QML for non-default installs.
    Q_INVOKABLE void setCliPath(const QString& path);

private:
    QString run(const QStringList& args);
    // Set in the constructor to the `msig` shipped alongside this plugin; see
    // resolveCli() in the .cpp. Only overridden if QML calls setCliPath().
    QString m_cli;
};
