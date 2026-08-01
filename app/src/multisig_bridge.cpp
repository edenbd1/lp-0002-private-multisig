// SPDX-License-Identifier: MIT OR Apache-2.0
#include "multisig_bridge.h"

#include <QProcess>
#include <QStringList>

MultisigBridge::MultisigBridge(QObject* parent) : QObject(parent) {}

void MultisigBridge::setCliPath(const QString& path) { m_cli = path; }

QString MultisigBridge::run(const QStringList& args) {
    QProcess proc;
    proc.start(m_cli, args);
    if (!proc.waitForStarted(3000)) {
        return QStringLiteral("error: could not start '%1'. Set the CLI path.")
            .arg(m_cli);
    }
    // Generous: building a witness folds a Merkle path and re-verifies it
    // locally before emitting anything, which is fast, but a cold page cache on
    // a large member set is not.
    proc.waitForFinished(30000);
    const QString out = QString::fromUtf8(proc.readAllStandardOutput());
    const QString err = QString::fromUtf8(proc.readAllStandardError());
    if (proc.exitCode() != 0) {
        // The CLI's refusals are written to be read by a human — a double
        // approval, a swapped action, a short threshold all explain themselves.
        // Pass them through rather than replacing them with a generic message.
        return QStringLiteral("error: %1").arg(err.isEmpty() ? out : err);
    }
    return out.trimmed();
}

QString MultisigBridge::newMultisig(const QString& dir, int members,
                                    int threshold, const QString& idHex) {
    QStringList args{QStringLiteral("new-multisig"),
                     QStringLiteral("--members"), QString::number(members),
                     QStringLiteral("--threshold"), QString::number(threshold),
                     QStringLiteral("--out"), dir};
    if (!idHex.isEmpty()) {
        args << QStringLiteral("--id") << idHex;
    }
    return run(args);
}

QString MultisigBridge::propose(const QString& dir, const QString& proposalId,
                                const QString& action) {
    return run(QStringList{QStringLiteral("propose"),
                           QStringLiteral("--dir"), dir,
                           QStringLiteral("--proposal-id"), proposalId,
                           QStringLiteral("--action"), action});
}

QString MultisigBridge::approve(const QString& dir, const QString& proposalId,
                                int memberIndex, const QString& mskHex,
                                const QString& outPath) {
    QStringList args{QStringLiteral("approve-args"),
                     QStringLiteral("--dir"), dir,
                     QStringLiteral("--proposal-id"), proposalId,
                     QStringLiteral("--out"), outPath};
    // Exactly one of the two, mirroring the CLI's own constraint.
    if (!mskHex.isEmpty()) {
        args << QStringLiteral("--msk") << mskHex;
    } else {
        args << QStringLiteral("--member") << QString::number(memberIndex);
    }
    return run(args);
}

QString MultisigBridge::status(const QString& dir, const QString& proposalId) {
    return run(QStringList{QStringLiteral("status"),
                           QStringLiteral("--dir"), dir,
                           QStringLiteral("--proposal-id"), proposalId});
}

QString MultisigBridge::executeArgs(const QString& dir,
                                    const QString& proposalId,
                                    const QString& outPath) {
    return run(QStringList{QStringLiteral("execute-args"),
                           QStringLiteral("--dir"), dir,
                           QStringLiteral("--proposal-id"), proposalId,
                           QStringLiteral("--out"), outPath});
}
