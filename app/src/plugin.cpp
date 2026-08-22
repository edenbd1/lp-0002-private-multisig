// SPDX-License-Identifier: MIT OR Apache-2.0
#include "plugin.h"
#include "multisig_bridge.h"

#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickWidget>
#include <QUrl>

MultisigPlugin::MultisigPlugin(QObject* parent) : QObject(parent) {}

MultisigPlugin::~MultisigPlugin() = default;

QWidget* MultisigPlugin::createWidget(LogosAPI* /*api*/) {
    // The bridge shells out to the local `msig` CLI to drive the multisig
    // lifecycle; submission to the chain is via `spel` on the same host. The
    // GUI never holds a member secret.
    m_bridge = new MultisigBridge(this);

    // Qt's resource system is process-global: two modules that both register
    // /qml/Main.qml resolve to whichever registered first, so with a second
    // module installed one tile drew the other's UI over its own data.
    // The prefix is this module's name, which cannot collide.
    auto* view = new QQuickWidget();
    view->engine()->rootContext()->setContextProperty(
        QStringLiteral("bridge"), m_bridge);
    view->setResizeMode(QQuickWidget::SizeRootObjectToView);
    view->setSource(QUrl(QStringLiteral("qrc:/lp_0002_multisig/Main.qml")));
    return view;
}

void MultisigPlugin::destroyWidget(QWidget* widget) {
    if (widget) {
        widget->deleteLater();
    }
}
