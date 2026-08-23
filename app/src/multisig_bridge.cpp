// SPDX-License-Identifier: MIT OR Apache-2.0
#include "multisig_bridge.h"

#include <QFileInfo>
#include <QProcess>
#include <QStringList>

#include <dlfcn.h>

namespace {

// Where the CLI is, without being told.
//
// The `.lgx` ships `msig` in the same directory as this plugin, but Basecamp is
// launched from the Finder, so it inherits a login PATH that contains neither
// the user's cargo bin nor the module directory. Looking up a bare "msig" would
// therefore fail on the first button press of a freshly installed package —
// the package would load and then do nothing, which is the worst of both.
//
// dladdr gives us the path of the binary this code lives in, so the sibling is
// resolvable at runtime on both macOS and Linux without hardcoding anything.
QString resolveCli() {
    Dl_info info{};
    if (dladdr(reinterpret_cast<const void*>(&resolveCli), &info) && info.dli_fname) {
        const QFileInfo self(QString::fromUtf8(info.dli_fname));
        const QString sibling = self.absolutePath() + QStringLiteral("/msig");
        if (QFileInfo(sibling).isExecutable()) {
            return sibling;
        }
    }
    // Developer builds run the plugin out of a build tree with no CLI beside
    // it; there, PATH is the right answer.
    return QStringLiteral("msig");
}

}  // namespace

MultisigBridge::MultisigBridge(QObject* parent)
    : QObject(parent), m_cli(resolveCli()) {}

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
                                const QString& recipientHex,
                                const QString& amount, const QString& memo) {
    return run(QStringList{QStringLiteral("propose"),
                           QStringLiteral("--dir"), dir,
                           QStringLiteral("--proposal-id"), proposalId,
                           QStringLiteral("--recipient"), recipientHex,
                           QStringLiteral("--amount"), amount,
                           QStringLiteral("--memo"), memo});
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
