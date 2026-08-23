// SPDX-License-Identifier: MIT OR Apache-2.0
//
// LP-0002 private multisig surface for Basecamp.
//
// The flow mirrors the CLI exactly, because it *is* the CLI: every button runs
// an `msig` subcommand through MultisigBridge. What the GUI shows and what the
// chain verifies therefore cannot drift apart.
//
// The design point worth noticing: nothing in this window ever displays which
// member approved. The approval list shows marker addresses, because that is
// all the chain records, and all the other members can see.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    color: "#0f1115"
    implicitWidth: 900
    implicitHeight: 700

    property string workDir: ""
    property string proposalId: "0000000000000000000000000000000000000000000000000000000000000001"

    function log(text) {
        output.text = text + "\n\n" + output.text
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 14

        Label {
            text: "Private M-of-N Multisig"
            color: "#e8eaed"
            font.pixelSize: 22
            font.bold: true
        }
        Label {
            text: "Approve a proposal by proving you are one of the members — without revealing which one, to observers or to the other members."
            color: "#9aa0a6"
            font.pixelSize: 13
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        // -- working directory ------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Label { text: "Multisig folder"; color: "#9aa0a6"; Layout.preferredWidth: 120 }
            TextField {
                id: dirField
                Layout.fillWidth: true
                placeholderText: "/path/to/multisig-dir"
                onTextChanged: root.workDir = text
            }
            TextField {
                id: cliField
                Layout.preferredWidth: 160
                placeholderText: "msig binary"
                onTextChanged: if (text.length > 0) bridge.setCliPath(text)
            }
        }

        // -- create -----------------------------------------------------------
        GroupBox {
            title: "1. Create"
            Layout.fillWidth: true
            RowLayout {
                spacing: 8
                Label { text: "members"; color: "#9aa0a6" }
                SpinBox { id: membersBox; from: 1; to: 64; value: 5 }
                Label { text: "threshold"; color: "#9aa0a6" }
                SpinBox { id: thresholdBox; from: 1; to: 64; value: 3 }
                Button {
                    text: "New multisig"
                    enabled: root.workDir.length > 0
                    onClicked: root.log(bridge.newMultisig(
                        root.workDir, membersBox.value, thresholdBox.value, ""))
                }
            }
        }

        // -- propose ----------------------------------------------------------
        GroupBox {
            title: "2. Propose"
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                spacing: 8
                RowLayout {
                    Layout.fillWidth: true
                    Label { text: "proposal id"; color: "#9aa0a6"; Layout.preferredWidth: 90 }
                    TextField {
                        id: propIdField
                        Layout.fillWidth: true
                        text: root.proposalId
                        onTextChanged: root.proposalId = text
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    Label { text: "pay"; color: "#9aa0a6"; Layout.preferredWidth: 90 }
                    TextField {
                        id: recipientField
                        Layout.fillWidth: true
                        placeholderText: "recipient account id, 64 hex characters"
                    }
                    TextField {
                        id: amountField
                        Layout.preferredWidth: 110
                        placeholderText: "amount"
                        // A u128 on chain, so it travels as text: QML numbers are
                        // doubles and a large amount would arrive rounded.
                        validator: RegularExpressionValidator {
                            regularExpression: /[1-9][0-9]*/
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    Label { text: "memo"; color: "#9aa0a6"; Layout.preferredWidth: 90 }
                    TextField {
                        id: actionField
                        Layout.fillWidth: true
                        placeholderText: "transfer 250 LEZ to the grants treasury"
                    }
                    Button {
                        text: "Bind"
                        enabled: root.workDir.length > 0
                                 && recipientField.text.length === 64
                                 && amountField.text.length > 0
                                 && actionField.text.length > 0
                        onClicked: root.log(bridge.propose(
                            root.workDir, root.proposalId,
                            recipientField.text, amountField.text,
                            actionField.text))
                    }
                }
                Label {
                    text: "The recipient, the amount and the memo are all bound into the proposal reference. Re-binding the same id to a different payment is refused: the approvals already gathered would not carry over to it. Reaching the threshold moves the amount out of the multisig's treasury."
                    color: "#6b7076"
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }

        // -- approve ----------------------------------------------------------
        GroupBox {
            title: "3. Approve"
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                spacing: 8
                RowLayout {
                    spacing: 8
                    Label { text: "member #"; color: "#9aa0a6" }
                    SpinBox { id: memberBox; from: 0; to: 63; value: 0 }
                    Label { text: "or secret"; color: "#9aa0a6" }
                    TextField {
                        id: mskField
                        Layout.fillWidth: true
                        placeholderText: "your msk in hex (never leaves this machine)"
                        echoMode: TextInput.Password
                    }
                    Button {
                        text: "Build approval"
                        enabled: root.workDir.length > 0
                        onClicked: root.log(bridge.approve(
                            root.workDir, root.proposalId, memberBox.value,
                            mskField.text, root.workDir + "/approve.args"))
                    }
                }
                Label {
                    text: "Submit the emitted arguments with spel on the privacy-preserving path. Proving takes about two and a half minutes; the approval is a real Risc0 proof that LEZ's privacy circuit verifies on chain."
                    color: "#6b7076"
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }

        // -- status and execute ------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Button {
                text: "Status"
                enabled: root.workDir.length > 0
                onClicked: root.log(bridge.status(root.workDir, root.proposalId))
            }
            Button {
                text: "Build execution"
                enabled: root.workDir.length > 0
                onClicked: root.log(bridge.executeArgs(
                    root.workDir, root.proposalId, root.workDir + "/exec.args"))
            }
            Item { Layout.fillWidth: true }
        }

        // -- output ------------------------------------------------------------
        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            TextArea {
                id: output
                readOnly: true
                color: "#c8ccd0"
                font.family: "Menlo, Monaco, monospace"
                font.pixelSize: 12
                wrapMode: TextArea.Wrap
                background: Rectangle { color: "#16181d"; radius: 4 }
                text: "Point the folder field at a multisig directory to begin."
            }
        }
    }
}
