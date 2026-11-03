// Tests for the comments panel (the agent conversation), its composer and connections.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "device_frame.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "player_overlay.h"
#include "presets_panel.h"
#include "program_level.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "workspace.h"

#include <QAbstractItemView>
#include <QAccessible>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMenuBar>
#include <QMimeData>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QScreen>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QWheelEvent>
#include <QWidget>

#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockAreaWidget.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>
#include <ElidingLabel.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

class CommentsTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void inboundMcpConfigValidatesStdioShape() {
        QVERIFY(!validateInboundMcpConfig("[]").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{}").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{broken").isEmpty());
        const QByteArray remote = R"({"mcpServers":{"remote":{"url":"https://example.com/mcp"}}})";
        const QByteArray emptyCommand = R"({"mcpServers":{"local":{"command":" "}}})";
        const QByteArray invalidArgs = R"({"mcpServers":{"local":{"command":"server","args":[1]}}})";
        const QByteArray invalidEnv = R"({"mcpServers":{"local":{"command":"server","env":{"TOKEN":1}}}})";
        const QByteArray valid = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"],"env":{"TOKEN":"value"},"tools":["search"]}}})";
        QVERIFY(validateInboundMcpConfig(remote).contains(QStringLiteral("URL-only")));
        QVERIFY(!validateInboundMcpConfig(emptyCommand).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidArgs).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidEnv).isEmpty());
        QVERIFY(validateInboundMcpConfig(valid).isEmpty());
    }

    void inboundMcpImportRequiresSaveAndInvalidSavePreservesConfig() {
        QTemporaryDir project;
        QVERIFY(project.isValid());
        QVERIFY(QDir(project.path()).mkpath(QStringLiteral(".cursor")));
        const QByteArray config = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"]}}})";
        QFile source(QDir(project.path()).filePath(QStringLiteral(".cursor/mcp.json")));
        QVERIFY(source.open(QIODevice::WriteOnly));
        QCOMPARE(source.write(config), qint64(config.size()));
        source.close();
        const QString savedPath = QDir(project.path()).filePath(QStringLiteral(".kerf/mcp.json"));
        MainWindow window{project.path(), QString()};
        const bool originalEnabled = window.m_mcpEnabled->isChecked();
        window.m_mcpEnabled->setChecked(!originalEnabled);
        QCOMPARE(QSettings().value(QStringLiteral("agent/mcpEnabled")).toBool(), !originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpEnabled")).toBool(), !originalEnabled);
        window.m_mcpEnabled->setChecked(originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(0));
        auto *load = window.findChild<QPushButton *>(QStringLiteral("loadProjectCursorMcp"));
        QVERIFY(load);
        QVERIFY(window.findChild<QPushButton *>(QStringLiteral("loadHomeCursorMcp")));
        load->click();
        QCOMPARE(window.m_inboundMcpJson->toPlainText().toUtf8(), config);
        QVERIFY(!QFileInfo::exists(savedPath));
        auto *save = window.findChild<QPushButton *>(QStringLiteral("saveInboundMcpConfig"));
        QVERIFY(save);
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QFile saved(savedPath);
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        saved.close();
        window.m_inboundMcpJson->setPlainText(QStringLiteral("[]"));
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        MainWindow reopened{project.path(), QString()};
        QCOMPARE(reopened.m_inboundMcpJson->toPlainText().toUtf8(), config);
    }

    void mcpConfigCopiesProjectWithoutCredentials() {
        const QString projectPath = QDir::temp().filePath(QStringLiteral("Kerf project \"quoted\""));
        MainWindow window{projectPath, QString()};
        auto *section = window.findChild<QGroupBox *>(QStringLiteral("mcpSection"));
        QVERIFY(section);
        auto *editor = section->findChild<QTextEdit *>(QStringLiteral("mcpConfigJson"));
        auto *button = section->findChild<QPushButton *>(QStringLiteral("copyMcpConfig"));
        QVERIFY(editor);
        QVERIFY(button);
        QVERIFY(editor->isReadOnly());
        const QJsonObject expected{{QStringLiteral("mcpServers"), QJsonObject{
            {QStringLiteral("kerf"), QJsonObject{
                {QStringLiteral("command"), mcpExecutable(projectPath)},
                {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
            }}}}};
        QCOMPARE(QJsonDocument::fromJson(editor->toPlainText().toUtf8()).object(), expected);
        button->click();
        QCOMPARE(QApplication::clipboard()->text(), editor->toPlainText());
        QCOMPARE(section->findChild<QLabel *>(QStringLiteral("muted"))->text(),
                 QStringLiteral("MCP config copied"));
    }

    void agentPayloadFollowsSettingsProtocol() {
        MainWindow window{QString(), QString()};
        showAssistant(window);
        QVERIFY(!window.findChild<QComboBox *>(QStringLiteral("agentProvider")));
        QVERIFY(!window.findChild<QFrame *>(QStringLiteral("agentContext")));
        QVERIFY(!window.findChild<QLabel *>(QStringLiteral("agentConfig")));
        QVERIFY(window.findChild<QComboBox *>(QStringLiteral("agentModel")));
        // Provider details live in Settings, not beside the chat.
        QWidget *chatTab = window.findChild<QWidget *>(QStringLiteral("chatPanel"));
        QVERIFY(chatTab);
        QVERIFY(window.m_preferences->isAncestorOf(window.m_anthropicKey));
        for (auto *label : chatTab->findChildren<QLabel *>()) {
            QVERIFY(!label->text().contains(QStringLiteral("OpenAI")));
            QVERIFY(!label->text().contains(QStringLiteral("Anthropic")));
        }
        auto *anthropicRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveAnthropic"));
        auto *openaiRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveOpenai"));
        QVERIFY(anthropicRadio);
        QVERIFY(openaiRadio);
        const QSignalBlocker anthropicBlock(anthropicRadio);
        const QSignalBlocker openaiBlock(openaiRadio);
        // Keys of one's own are the Advanced connection.
        QVERIFY(window.m_advancedSettings->isAncestorOf(anthropicRadio));
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }

        const QString anthropicKey = QStringLiteral("sk-ant-test");
        const QString anthropicUrl = QStringLiteral("https://anthropic.example/v1");
        const QString anthropicModel = QStringLiteral("claude-test-model");
        anthropicRadio->setChecked(true);
        window.m_anthropicKey->setText(anthropicKey);
        window.m_anthropicBaseUrl->setText(anthropicUrl);
        window.m_anthropicModel->setText(anthropicModel);
        window.updateAgentModelChoices();
        const QJsonObject anthropicPayload = window.agentSettingsPayload();
        QCOMPARE(anthropicPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("anthropic"));
        QCOMPARE(anthropicPayload.value(QStringLiteral("apiKey")).toString(), anthropicKey);
        QCOMPARE(anthropicPayload.value(QStringLiteral("baseUrl")).toString(), anthropicUrl);
        QCOMPARE(anthropicPayload.value(QStringLiteral("model")).toString(), anthropicModel);

        const QString openaiKey = QStringLiteral("sk-openai-test");
        const QString openaiUrl = QStringLiteral("https://openai.example/v1");
        const QString openaiModel = QStringLiteral("gpt-test-model");
        openaiRadio->setChecked(true);
        window.m_openaiKey->setText(openaiKey);
        window.m_openaiBaseUrl->setText(openaiUrl);
        window.m_openaiModel->setText(openaiModel);
        window.updateAgentModelChoices();
        const QJsonObject openaiPayload = window.agentSettingsPayload();
        QCOMPARE(openaiPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("openai"));
        QCOMPARE(openaiPayload.value(QStringLiteral("apiKey")).toString(), openaiKey);
        QCOMPARE(openaiPayload.value(QStringLiteral("baseUrl")).toString(), openaiUrl);
        QCOMPARE(openaiPayload.value(QStringLiteral("model")).toString(), openaiModel);
    }

    void editorChromeKeepsComposerAtTheBottom() {
        MainWindow window{QString(), QString()};
        window.show();
        showAssistant(window);
        QTest::qWait(20);

        // The status bar carries live state instead of sitting empty.
        QVERIFY(window.statusBar()->isVisible());
        QCOMPARE(window.m_statusConnection->text(), QStringLiteral("● Connecting…"));
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(window.m_statusConnection->text(), QStringLiteral("● Connected"));
        // The player's controls on the picture follow the connection.
        const auto controls = {PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack, PlayerOverlay::Control::StepForward};
        PlayerOverlay *overlay = window.m_viewer->overlay();
        QVERIFY(std::all_of(controls.begin(), controls.end(), [overlay](PlayerOverlay::Control c) { return overlay->controlEnabled(c); }));
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(std::none_of(controls.begin(), controls.end(), [overlay](PlayerOverlay::Control c) { return overlay->controlEnabled(c); }));
        // No strip of transport buttons under the viewer any more.
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("viewerFrame"))->findChild<QWidget *>(QStringLiteral("transport")));
        // The conversation is no dock of its own: it lives in the Program
        // monitor's stage, beside the picture.
        QVERIFY(!window.m_workspace->panel(QStringLiteral("assistant")));
        QCOMPARE(window.m_chat->parentWidget(), window.m_viewer);
        QVERIFY(window.m_chat->isVisible());

        // The composer is the panel's footer; the status line sits above it.
        auto *composer = window.findChild<QFrame *>(QStringLiteral("chatComposer"));
        QVERIFY(composer);
        QCOMPARE(composer->parentWidget(), window.m_chat);
        QCOMPARE(composer->geometry().bottom(), window.m_chat->rect().bottom());
        QVERIFY(window.m_chat->statusRow()->isVisible());
        QVERIFY(window.m_chat->statusRow()->mapTo(window.m_chat, QPoint()).y() < composer->y());
        // Export progress is the window's, in the status bar.
        QVERIFY(window.statusBar()->isAncestorOf(window.m_exportProgress));
        QVERIFY(window.m_exportProgress->isHidden());

        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"progress", 0.5}});
        QVERIFY(!window.m_exportProgress->isHidden());
        QCOMPARE(window.m_exportProgress->value(), 50);
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"progress", 1.0}});
        QVERIFY(window.m_exportProgress->isHidden());
    }

    void chatComposerSupportsMultilineWithoutAccidentalSend() {
        MainWindow window{QString(), QString()};
        window.show();
        showAssistant(window);
        window.m_chatInput->setEnabled(true);
        window.m_chatInput->setFocus();
        window.m_chatInput->setPlainText(QStringLiteral("First edit"));
        window.m_chatInput->moveCursor(QTextCursor::End);
        QTest::keyClick(window.m_chatInput, Qt::Key_Return, Qt::ShiftModifier);
        QTest::keyClicks(window.m_chatInput, "Second edit");
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        // Return must preserve a draft while sending is unavailable.
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        QVERIFY(!window.m_sendButton->isEnabled());
        QVERIFY(window.m_abortButton->isHidden());
    }

    // Every turn is a comment row, Kerf's and the reader's alike, as in a
    // video's comments panel: an avatar, the name and time, the text. Kerf's
    // steps fold under its comment as replies; streamed text stays in its
    // one comment; the view follows the latest unless scrolled away.
    void conversationIsCommentRowsAndKeepsStreamInItsComment() {
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        showAssistant(window);
        ChatPanel *chat = window.m_chat;
        // Before the first message, Kerf's welcome with requests to try, in
        // colours that read on the panel's surface, in both appearances.
        QVERIFY(chat->welcome()->isVisible());
        {
            {
                QString words;
                int colours = 0;
                for (QLabel *label : chat->welcome()->findChildren<QLabel *>()) {
                    label->ensurePolished();
                    words += label->text() + QLatin1Char('\n');
                    const QColor ink = label->palette().color(QPalette::WindowText);
                    const qreal ratio = contrastRatio(ink, editorTheme().chatSurface);
                    QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1 on %2: %3").arg(ink.name(), editorTheme().chatSurface.name()).arg(ratio, 0, 'f', 2)));
                    ++colours;
                }
                QVERIFY(colours >= 4);
                // A comment as every other: name and time, one plain line, then the examples.
                QVERIFY(words.startsWith(QStringLiteral("Kerf\nnow\n")));
                QVERIFY(words.contains(QStringLiteral("review it before it touches the cut")));
                QVERIFY(words.contains(QStringLiteral("Try asking")));
                QVERIFY(!words.contains(QStringLiteral("Make your next cut.")));
            }
        }
        // An example fills the composer and sends nothing.
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(chat->examples().size(), 2);
        chat->examples().constFirst()->click();
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Trim the opening to three seconds."));
        QVERIFY(window.m_lastCommand.value("type").toString() != QStringLiteral("agent"));
        window.m_chatInput->clear();

        // A step before Kerf has written waits, and adds no row.
        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Inspecting timeline"));
        QVERIFY(chat->isEmpty());
        const int activityCount = window.m_activityList->count();

        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim <intro>\nKeep the dialogue."));
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "I’ll trim "}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "Reading <timeline>"}});
        window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "failed"}, {"message", "Could not read <media>"}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "the opening."}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "I’ll trim the opening."}});
        QCoreApplication::processEvents();

        const QString transcript = chat->transcript();
        QVERIFY(!chat->welcome()->isVisible());
        QVERIFY(transcript.contains(QStringLiteral("Trim <intro>\nKeep the dialogue.")));
        QCOMPARE(transcript.count(QStringLiteral("I’ll trim the opening.")), 1);
        QVERIFY(transcript.contains(QStringLiteral("Could not read <media>")));
        QVERIFY(!transcript.contains(QStringLiteral("Reading <timeline>")));
        QCOMPARE(window.m_activityList->count(), activityCount + 1);
        const QList<CommentRow *> rows = chat->rows();
        QCOMPARE(rows.size(), 3);
        QCOMPARE(rows.at(0)->kind(), CommentRow::Kind::User);
        QCOMPARE(rows.at(1)->kind(), CommentRow::Kind::Kerf);
        QCOMPARE(rows.at(2)->kind(), CommentRow::Kind::Error);
        QCOMPARE(rows.at(0)->name(), QStringLiteral("You"));
        QCOMPARE(rows.at(1)->name(), QStringLiteral("Kerf"));
        // The reader's initial on a neutral disc; Kerf's monogram on its purple.
        QCOMPARE(rows.at(0)->avatar()->who(), ChatAvatar::Who::User);
        QCOMPARE(rows.at(1)->avatar()->who(), ChatAvatar::Who::Kerf);
        QCOMPARE(rows.at(1)->avatar()->monogram(), QStringLiteral("K"));
        // No bubbles: every row starts at the panel's padding, the reader's too.
        for (const CommentRow *row : rows) {
            QCOMPARE(row->avatar()->mapTo(chat, QPoint()).x(), ChatPanel::kSidePadding);
            QCOMPARE(row->avatar()->width(), ChatPanel::kAvatar);
            // Text comes literally, never as markup.
            QVERIFY(row->body()->textFormat() == Qt::RichText);
        }
        QVERIFY(rows.at(0)->body()->text().contains(QStringLiteral("Trim &lt;intro&gt;")));
        // The tool's step is a reply under Kerf's comment, folded behind "1 step".
        QCOMPARE(rows.at(1)->replies(), QStringList{QStringLiteral("Reading <timeline>")});
        QVERIFY(rows.at(1)->expander()->isVisible());
        QCOMPARE(rows.at(1)->expander()->text(), QStringLiteral("1 step"));
        QVERIFY(!rows.at(1)->replyList()->isVisible());
        rows.at(1)->expander()->click();
        QVERIFY(rows.at(1)->repliesExpanded() && rows.at(1)->replyList()->isVisible());
        QVERIFY(!rows.at(0)->expander()->isVisible());
        // The reader's rows can be asked again; every row can be copied.
        QCOMPARE(rows.at(0)->actions().size(), 2);
        QCOMPARE(rows.at(1)->actions().size(), 1);
        rows.at(1)->actions().constFirst()->click();
        QCOMPARE(QApplication::clipboard()->text(), QStringLiteral("I’ll trim the opening."));
        // The header counts the messages, as a comments panel does, and so
        // does the rail's chat button.
        QCOMPARE(chat->countLabel()->text(), QStringLiteral("3"));
        QCOMPARE(window.m_rail->chatButton()->label(), QStringLiteral("3"));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("A separate response."));
        QCOMPARE(chat->rows().size(), 4);
        QCOMPARE(chat->countLabel()->text(), QStringLiteral("4"));

        // New output must not pull the reader away from earlier messages.
        for (int i = 0; i < 12; ++i)
            window.appendConversation(QStringLiteral("You"), QStringLiteral("A longer edit request.\nKeep this line readable."));
        QCoreApplication::processEvents();
        auto *scroll = chat->scroll()->verticalScrollBar();
        QVERIFY(scroll->maximum() > 0);
        // It followed the output to the latest.
        QCOMPARE(scroll->value(), scroll->maximum());
        scroll->setValue(0);
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("New response."));
        QCoreApplication::processEvents();
        QCOMPARE(scroll->value(), 0);
        // Nothing in it scrolls sideways.
        QCOMPARE(chat->scroll()->widget()->width(), chat->scroll()->viewport()->width());
    }

    void composerStatesAndNarrowLayoutRemainUsable() {
        MainWindow window{QString(), QString()};
        window.resize(1080, 700);
        window.show();
        showAssistant(window);
        ChatPanel *chat = window.m_chat;
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("offline"));
        // Offline, the composer is one quiet line, with nothing to send.
        QVERIFY(!chat->composerExpanded());
        window.setConnection(QStringLiteral("ready"));
        window.activateWindow();
        // At rest: one line, no buttons, as a comment field before it is used.
        QVERIFY(!chat->composerExpanded());
        window.m_chatInput->setFocus();
        QTRY_COMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("focused"));
        // In focus its row of the model, Cancel and Send shows, and its
        // underline turns the accent, with no ring around the field.
        QVERIFY(chat->composerExpanded());
        QVERIFY(chat->cancelButton()->isVisible() && window.m_sendButton->isVisible());
        QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("focusHalo")));
        auto *rule = window.m_chatComposer->findChild<QWidget *>(QStringLiteral("chatComposerRule"));
        QVERIFY(rule);
        const auto underline = [&] {
            const QImage composer = window.m_chatComposer->grab().toImage();
            const QPoint at = rule->mapTo(window.m_chatComposer, QPoint(rule->width() / 2, 0));
            return composer.pixelColor(at * composer.devicePixelRatio()).rgb();
        };
        QCOMPARE(underline(), editorTheme().chatComposerRuleFocus.rgb());
        QCOMPARE(editorTheme().chatComposerRuleFocus.rgb(), editorTheme().focusRing.rgb());
        // So it does as the model's field is typed in (with keys of one's
        // own), and it goes back as the focus leaves the composer.
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }
        window.updateAgentModelChoices();
        QVERIFY(window.m_modelSelect->isEnabled());
        QTest::mouseClick(window.m_modelSelect->lineEdit(), Qt::LeftButton);
        QVERIFY(window.m_chatComposer->isAncestorOf(QApplication::focusWidget()));
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("focused"));
        QCOMPARE(underline(), editorTheme().chatComposerRuleFocus.rgb());
        QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("focusHalo")));
        window.m_timeline->setFocus();
        QVERIFY(!window.m_chatComposer->isAncestorOf(QApplication::focusWidget()));
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("ready"));
        QVERIFY(!chat->composerExpanded());
        QVERIFY(underline() != editorTheme().chatComposerRuleFocus.rgb());
        // A draft keeps it open, and Kerf at work shows Stop for Send.
        window.m_chatInput->setPlainText(QStringLiteral("Keep my draft"));
        QVERIFY(chat->composerExpanded());
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("sending"));
        QVERIFY(!window.m_chatInput->isEnabled());
        QVERIFY(window.m_abortButton->isVisible());
        QVERIFY(!window.m_sendButton->isVisible());
        // The status line says so, with three dots, after the latest comment.
        QVERIFY(chat->statusRow()->isVisible());
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Thinking"));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QVERIFY(window.m_chatInput->isEnabled());
        QVERIFY(window.m_sendButton->isEnabled());
        QVERIFY(!chat->statusRow()->isVisible());
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Keep my draft"));
        // Cancel clears the draft and folds the composer.
        chat->cancelButton()->click();
        QVERIFY(window.m_chatInput->toPlainText().isEmpty());
        QVERIFY(!chat->composerExpanded());
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim the opening to three seconds."));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I’ll shorten the opening and keep the rest of your sequence in place.\nReview the changes before applying them."));
        window.appendConversationNote(QStringLiteral("Tool"), QString(500, QLatin1Char('x')));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "review"},
            {"script", "trim c1 out -1s\n"}, {"documentDiff", "-    clip c1 dur=00:00:04:00\n+    clip c1 dur=00:00:03:00\n"}});
        chat->setProposalExpanded(true);
        window.m_chatInput->setFocus();
        QCoreApplication::processEvents();
        QVERIFY(window.m_chatInput->height() <= 36);
        // In the smallest window the panel is short: the proposal scrolls
        // with the list it heads, so its script opens in place and the
        // composer stays whole at the panel's foot; Accept is in reach on
        // the proposal in view, or else on the compact bar under the header.
        QVERIFY(chat->height() < ViewerWidget::kMinChatHeight);
        QVERIFY(chat->proposalExpanded() && window.m_proposalDiff->isVisible() && chat->proposalExpander()->isVisible());
        QVERIFY(chat->rect().contains(window.m_chatComposer->geometry()));
        QCOMPARE(window.m_chatComposer->geometry().bottom(), chat->rect().bottom());
        QTRY_VERIFY(window.m_proposalBox->rect().contains(window.m_acceptButton->geometry()));
        {
            const QRect pills(window.m_acceptButton->mapTo(chat->scroll()->widget(), QPoint()), window.m_acceptButton->size());
            const int top = chat->scroll()->verticalScrollBar()->value();
            const bool inView = pills.top() >= top && pills.bottom() < top + chat->scroll()->viewport()->height();
            QCOMPARE(chat->proposalBar()->isVisible(), !inView);
        }
        QVERIFY(chat->scroll()->mapTo(chat, QPoint(0, chat->scroll()->height())).y() <= window.m_chatComposer->y());
        // The long step wraps inside the column rather than widening it.
        QCOMPARE(chat->scroll()->widget()->width(), chat->scroll()->viewport()->width());
        // In a taller window the script opens under the pills.
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        QVERIFY(window.m_proposalDiff->isVisible());
        QVERIFY(window.m_proposalBox->rect().contains(window.m_proposalDiff->geometry()));
        QVERIFY(window.m_proposalDiff->geometry().top() > window.m_acceptButton->geometry().bottom());
        QCOMPARE(window.m_chatComposer->geometry().bottom(), chat->rect().bottom());
        // Optional artifact for manual review of the real styled widget tree.
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) QVERIFY(window.grab().save(screenshot));
    }

    // A conversation closed when the editor last closed is closed the next
    // time, and opens at the width it had; a width kept while it was a
    // docked column carries over the first time. (In a window wide enough
    // that the wide picture's laptop leaves it its own width.)
    void conversationKeepsItsWidthAndStateAfterARelaunch() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        {
            QSettings().setValue(QStringLiteral("workspace/assistantWidth"), 380);
            MainWindow window{QString(), QString()};
            window.resize(1680, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCOMPARE(window.m_chat->width(), 380);
            QTest::mousePress(window.m_chat->resizeHandle(), Qt::LeftButton, {}, QPoint(2, 60));
            QTest::mouseMove(window.m_chat->resizeHandle(), QPoint(2 - 20, 60));
            QTest::mouseRelease(window.m_chat->resizeHandle(), Qt::LeftButton, {}, QPoint(2 - 20, 60));
            QCoreApplication::processEvents();
            QCOMPARE(window.m_chat->width(), 400);
            window.setChatOpen(false);
            window.close();
        }
        MainWindow window{QString(), QString()};
        window.resize(1680, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QVERIFY(!window.chatOpen() && !window.m_chat->isVisible());
        QVERIFY(!window.m_chatToggle->isChecked() && !window.m_rail->chatButton()->isChecked());
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QCOMPARE(window.m_chat->width(), 400);
        QCOMPARE(window.m_chat->geometry(), window.m_viewer->stageLayout().panel);
    }

    // Nothing in the conversation's panel widens it or scrolls it sideways,
    // at its default width and at its narrowest, except Kerf's next steps:
    // one row of chips that scrolls sideways, as a chip bar does, rather
    // than wrap and take the conversation's height.
    void conversationFitsItsPanel() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        // Wide enough that a wide picture leaves the conversation its own width.
        window.resize(1680, 900);
        window.setConnection(QStringLiteral("ready"));
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                        QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music"}},
                                        QJsonObject{{"label", "Tighten the opening"}, {"request", "Trim the opening"}},
                                        QJsonObject{{"label", "Match the colour"}, {"request", "Match the shots"}}});
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "p1"},
                                          {"script", "trim v1b out -36f\nremove v1c\nadd title t2 at 00:00:12:00 text=\"A lower third that runs well past the column's edge\"\n"},
                                          {"documentDiff", "-    clip v1b dur=5s\n+    clip v1b dur=3.5s\n+    title t2 text=\"A lower third that runs well past the column's edge\"\n"}});
        window.m_chat->setProposalExpanded(true);
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("Done: /Users/editor/Movies/Harbor/footage/fisher-close-up-take-three-final.mov is shorter now."));
        QCoreApplication::processEvents();
        ChatPanel *column = window.m_chat;
        for (const int width : {ViewerWidget::kDefaultChatWidth, ViewerWidget::kMinChatWidth}) {
            window.m_viewer->setChatWidth(width);
            QCoreApplication::processEvents();
            QVERIFY(column->isVisible());
            QCOMPARE(column->width(), width);
            // The chips: one line, a fixed row's height, the first at the
            // panel's side padding; those past the edge fade under it.
            const auto chips = window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep"));
            QCOMPARE(chips.size(), 4);
            QSet<int> lines;
            for (QPushButton *chip : chips) {
                lines.insert(chip->y());
                QCOMPARE(chip->height(), ChatPanel::kChipHeight);
            }
            QCOMPARE(lines.size(), 1);
            QCOMPARE(window.m_chatNextSteps->height(), ChatPanel::kChipRowHeight);
            QCOMPARE(chips.constFirst()->x(), ChatPanel::kSidePadding);
            QPushButton *last = chips.constLast();
            QVERIFY(last->geometry().right() > window.m_chatNextSteps->width() - ChatPanel::kSidePadding);
            auto *trailing = window.m_chatNextSteps->findChild<QWidget *>(QStringLiteral("chipFadeTrailing"));
            auto *leading = window.m_chatNextSteps->findChild<QWidget *>(QStringLiteral("chipFadeLeading"));
            QVERIFY(trailing && leading);
            QTRY_VERIFY(trailing->isVisible());
            QVERIFY(!leading->isVisible());
            // Only chips that fit whole show: the trailing cover starts at the
            // first that does not, opaque (the panel's surface), so no sliver
            // of a cut chip shows beside the pager.
            {
                const int cover = trailing->x();
                QVERIFY(cover <= column->chipsForward()->x() - 8);
                bool hidden = false;
                for (QPushButton *chip : chips) {
                    QVERIFY2(chip->geometry().right() < cover || chip->x() >= cover,
                             qPrintable(QStringLiteral("%1 at %2..%3, cover at %4").arg(chip->text()).arg(chip->x()).arg(chip->geometry().right()).arg(cover)));
                    hidden = hidden || chip->x() >= cover;
                }
                QVERIFY(hidden);
                QVERIFY(chips.constFirst()->geometry().right() < cover);
                const QImage row = window.m_chatNextSteps->grab().toImage();
                const QPoint inCover(cover + 2, trailing->y() + trailing->height() / 2);
                QCOMPARE(row.pixelColor(inCover * row.devicePixelRatio()).rgb(), editorTheme().chatSurface.rgb());
            }
            // A wheel turned over the row scrolls it to its last chip, whole.
            for (int turn = 0; turn < 10; ++turn) {
                QWheelEvent wheel(QPointF(40, 20), window.m_chatNextSteps->mapToGlobal(QPointF(40, 20)), QPoint(), QPoint(0, -120),
                                  Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
                QCoreApplication::sendEvent(window.m_chatNextSteps, &wheel);
            }
            QCOMPARE(last->geometry().right(), window.m_chatNextSteps->width() - ChatPanel::kSidePadding - 1);
            QTRY_VERIFY(!trailing->isVisible() && leading->isVisible());
            for (QPushButton *chip : chips) QVERIFY(chip->geometry().right() < leading->width() || chip->x() >= leading->width());
            for (int turn = 0; turn < 10; ++turn) {
                QWheelEvent wheel(QPointF(40, 20), window.m_chatNextSteps->mapToGlobal(QPointF(40, 20)), QPoint(), QPoint(0, 120),
                                  Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
                QCoreApplication::sendEvent(window.m_chatNextSteps, &wheel);
            }
            QCOMPARE(chips.constFirst()->x(), ChatPanel::kSidePadding);
            // Round pagers over the fades, as a chip bar's: forward while the
            // row has more that way, back once it is scrolled; each pages it.
            ChatIconButton *back = column->chipsBack();
            ChatIconButton *forward = column->chipsForward();
            QTRY_VERIFY(forward->isVisible());
            QVERIFY(!back->isVisible());
            QCOMPARE(forward->size(), QSize(ChatPanel::kChipPager, ChatPanel::kChipPager));
            QVERIFY(forward->geometry().right() < window.m_chatNextSteps->width() - 4);
            QCOMPARE(ChatPanel::kChipFadeWidth, 48);
            forward->click();
            QVERIFY(chips.constFirst()->x() < ChatPanel::kSidePadding);
            QVERIFY(chips.constFirst()->x() >= ChatPanel::kSidePadding - ChatPanel::kChipPage);
            QVERIFY(back->isVisible());
            back->click();
            QCOMPARE(chips.constFirst()->x(), ChatPanel::kSidePadding);
            QVERIFY(!back->isVisible() && forward->isVisible());
            // The list scrolls with its scroll bar hidden (mockup-spec 4.5):
            // the bar takes no width, under the pointer or not, so the rows
            // keep the panel's whole width.
            {
                QScrollBar *listBar = window.m_chatScroll->verticalScrollBar();
                QVERIFY(listBar->maximum() > 0);
                QCOMPARE(listBar->value(), listBar->maximum());
                QCOMPARE(listBar->isVisible() ? listBar->width() : 0, 0);
                QEnterEvent enter(QPointF(20, 20), QPointF(20, 20), column->mapToGlobal(QPointF(20, 20)));
                QCoreApplication::sendEvent(column, &enter);
                QCoreApplication::processEvents();
                QCOMPARE(listBar->isVisible() ? listBar->width() : 0, 0);
                QEvent leave(QEvent::Leave);
                QCoreApplication::sendEvent(column, &leave);
            }
            // Nothing asks for more width than the panel has; a long path wraps.
            QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
            QCOMPARE(window.m_chatScroll->widget()->width(), window.m_chatScroll->viewport()->width());
            QVERIFY(window.m_chatComposer->minimumSizeHint().width() <= window.m_chatComposer->width());
            QVERIFY(!window.m_proposalDiff->horizontalScrollBar()->isVisible());
            for (const CommentRow *row : column->rows())
                QVERIFY(row->body()->mapTo(column, QPoint(row->body()->width(), 0)).x() <= column->width() - ChatPanel::kSidePadding);
            // The proposal's buttons and the composer's Send sit inside it.
            window.m_chatInput->setPlainText(QStringLiteral("Also duck the music"));
            QCoreApplication::processEvents();
            for (QWidget *control : std::initializer_list<QWidget *>{window.m_acceptButton, window.m_rejectButton, window.m_sendButton}) {
                QVERIFY(control->isVisible());
                QVERIFY(control->mapTo(column, QPoint(control->width(), 0)).x() <= column->width() - ChatPanel::kSidePadding);
            }
        }
    }

    void nextStepsAreButtonsThatAskForThem() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        // The last request was about clip c1.
        const QJsonObject focus{{"scope", "clip"}, {"clip", "c1"}, {"fromFrame", 0}, {"toFrame", 24}};
        window.m_lastFocus = focus;
        window.m_lastFocusLabel = QStringLiteral("clip c1");
        window.handleEvent(QJsonObject{{"type", "agent_next"}, {"steps", QJsonArray{
            QJsonObject{{"label", "Stronger glow"}, {"request", "Make the glow stronger."}},
            QJsonObject{{"label", "Wipe out"}, {"request", "Use a linear wipe into the next shot."}}}}});
        const auto chips = window.m_nextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep"));
        QCOMPARE(chips.size(), 2);
        QCOMPARE(chips.at(0)->text(), QStringLiteral("Stronger glow"));
        // The conversation, open beside the picture, shows them; the
        // timeline's bar shows them only while it is closed.
        QVERIFY(window.chatOpen());
        QVERIFY(window.m_aiBar->isHidden());
        window.setChatOpen(false);
        QVERIFY(!window.m_aiBar->isHidden());
        QCOMPARE(window.m_aiBarText->text(), QStringLiteral("Next with Kerf"));
        QCOMPARE(window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep")).size(), 2);
        // Pressing one asks for it, about what the last request was about.
        chips.at(1)->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value("text").toString(), QStringLiteral("Use a linear wipe into the next shot."));
        QCOMPARE(window.m_lastCommand.value("focus").toObject(), focus);
    }

    // Kerf's conversation, in its panel beside the picture, scrolls as a
    // comments panel does: the rows are cut cleanly at the edge under the
    // header (no cover, no fade, no blank band) and a hairline shows there
    // while the list is scrolled away from its top. The pinned proposal
    // heads the list and scrolls away with it; while its pills are out of
    // view, a compact bar under the header keeps its summary, Reject and
    // Accept in reach. Its script and diff are as tall as their lines, up
    // to ten.
    void conversationScrollsUnderACleanEdge() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
        });
        {
            const EditorTheme &theme = editorTheme();
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            QJsonObject proposal{{"status", "pending"}, {"proposalId", "p1"}, {"script", "trim v1b out -36f\nremove v1c\n"},
                                 {"documentDiff", "-    clip v1b dur=5s\n+    clip v1b dur=3.5s\n-    clip v1c\n"}};
            QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1080);
            timeline.insert(QStringLiteral("height"), 1920);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            snapshot.insert(QStringLiteral("pendingProposal"), proposal);
            window.applySnapshot(snapshot);
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            window.m_errorBanner->hide();
            for (int turn = 0; turn < 4; ++turn) {
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass, please."));
                window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I trimmed the close-up by 1.5 seconds and removed the drone pass."));
            }
            QCoreApplication::processEvents();
            ChatPanel *chat = window.m_chat;
            QScrollArea *scroll = window.m_chatScroll;
            QScrollBar *bar = scroll->verticalScrollBar();
            QWidget *content = scroll->widget();
            QFrame *sticky = chat->proposalBar();
            // The proposal heads the list, 16 points under its top, as a
            // pinned comment heads the comments.
            QCOMPARE(window.m_proposalBox->parentWidget(), content);
            QCOMPARE(window.m_proposalBox->y(), 16);
            // Followed to the latest, the proposal has scrolled away: the
            // compact bar holds its summary and pills, under the header.
            QTRY_VERIFY(bar->maximum() > 0);
            QTRY_COMPARE(bar->value(), bar->maximum());
            verifyListEnd(chat);
            QVERIFY(sticky->isVisible());
            QCOMPARE(sticky->y(), ChatPanel::kHeaderHeight);
            // What it does, without the count, where it fits whole; else only
            // the count, whole: never a list cut mid-way (its tip has the
            // summary and the count).
            QCOMPARE(chat->proposalBarSummary()->toolTip(), window.m_proposalSummary->text());
            QCOMPARE(chat->proposalBarSummary()->toolTip(), QStringLiteral("Trim, remove · 2 changes"));
            QVERIFY2(chat->proposalBarSummary()->text() == QStringLiteral("Trim, remove") ||
                         chat->proposalBarSummary()->text() == QStringLiteral("2 changes"),
                     qPrintable(chat->proposalBarSummary()->text()));
            QCOMPARE(chat->width(), ViewerWidget::kDefaultChatWidth);
            QCOMPARE(chat->proposalBarSummary()->text(), QStringLiteral("Trim, remove"));
            QVERIFY(QFontMetrics(chat->proposalBarSummary()->font()).horizontalAdvance(chat->proposalBarSummary()->text()) <=
                    chat->proposalBarSummary()->contentsRect().width());
            for (QPushButton *pill : {chat->barRejectButton(), chat->barAcceptButton()}) {
                QVERIFY(pill->isVisible() && pill->isEnabled());
                QVERIFY(sticky->rect().contains(pill->geometry()));
            }
            QVERIFY(chat->barRejectButton()->geometry().right() < chat->barAcceptButton()->geometry().left());
            // They follow the proposal's own pills.
            window.m_acceptButton->setEnabled(false);
            QVERIFY(!chat->barAcceptButton()->isEnabled());
            window.m_acceptButton->setEnabled(true);
            QVERIFY(chat->barAcceptButton()->isEnabled());
            // The hairline under the bar, and the rows cut cleanly under it.
            QImage image = window.grab().toImage();
            qreal ratio = image.devicePixelRatio();
            const QPoint edge = chat->listEdge()->mapTo(&window, QPoint(chat->width() / 2, 0));
            QCOMPARE(image.pixelColor(edge * ratio).rgb(), theme.divider.rgb());

            // Scrolled so a comment of the reader's is cut 10 points into
            // its avatar: the avatar runs right up to the view's edge.
            CommentRow *message = nullptr;
            for (CommentRow *row : chat->rows())
                if (row->kind() == CommentRow::Kind::User && row->y() > 200) {
                    message = row;
                    break;
                }
            QVERIFY(message);
            const int into = message->avatar()->mapTo(content, QPoint()).y() + 10;
            bar->setValue(into);
            QCoreApplication::processEvents();
            QCOMPARE(bar->value(), into);
            QVERIFY(sticky->isVisible());
            image = window.grab().toImage();
            ratio = image.devicePixelRatio();
            const QPoint viewTop = scroll->viewport()->mapTo(&window, QPoint());
            const QPoint inAvatar(message->avatar()->mapTo(&window, QPoint(5, 0)).x(), viewTop.y());
            QCOMPARE(image.pixelColor(inAvatar * ratio).rgb(), theme.chatAvatarUser.rgb());
            QCOMPARE(image.pixelColor((inAvatar - QPoint(0, 1)) * ratio).rgb(), theme.divider.rgb());

            // At the top: the proposal whole, no bar, no hairline.
            bar->setValue(0);
            QCoreApplication::processEvents();
            QVERIFY(!sticky->isVisible());
            QVERIFY(window.m_acceptButton->isVisible());
            QCOMPARE(window.m_proposalBox->mapTo(chat, QPoint()).y(), ChatPanel::kHeaderHeight + 1 + 16);
            QVERIFY(!chat->listEdge()->property("scrolled").toBool());
            image = window.grab().toImage();
            QCOMPARE(image.pixelColor(chat->listEdge()->mapTo(&window, QPoint(chat->width() / 2, 0)) * ratio).rgb(), theme.chatSurface.rgb());
            // Just past its head ("Pinned by Kerf", which says what the pills
            // decide), the bar comes, though the pills are still in view;
            // with its head in view, it goes.
            QWidget *head = window.m_proposalBox->findChild<QWidget *>(QStringLiteral("proposalPinLine"));
            QVERIFY(head);
            const int headBottom = head->mapTo(content, QPoint(0, head->height())).y();
            bar->setValue(headBottom + 1);
            QCoreApplication::processEvents();
            QVERIFY(sticky->isVisible());
            QVERIFY(window.m_acceptButton->mapTo(content, QPoint()).y() > bar->value());
            bar->setValue(headBottom);
            QCoreApplication::processEvents();
            QVERIFY(!sticky->isVisible());
            bar->setValue(0);
            QCoreApplication::processEvents();

            // The script, opened: as tall as its lines, no room left under its
            // last line, and no scroll bar.
            QTextEdit *diff = window.m_proposalDiff;
            QVERIFY(!diff->isVisible());
            chat->proposalExpander()->click();
            QCoreApplication::processEvents();
            QVERIFY(diff->isVisible());
            const int line = diff->fontMetrics().lineSpacing();
            const qreal lines = diff->document()->size().height();
            QVERIFY2(diff->viewport()->height() - lines < line,
                     qPrintable(QStringLiteral("%1 px for %2 px of lines").arg(diff->viewport()->height()).arg(lines)));
            QVERIFY(diff->viewport()->height() >= lines - 1);
            QCOMPARE(diff->verticalScrollBar()->maximum(), 0);
            // A long one stops at ten lines at most, and scrolls.
            QString longScript;
            for (int index = 0; index < 40; ++index) longScript += QStringLiteral("trim v%1 out -12f\n").arg(index);
            proposal.insert(QStringLiteral("script"), longScript);
            snapshot.insert(QStringLiteral("pendingProposal"), proposal);
            snapshot.insert(QStringLiteral("revision"), 4);
            window.applySnapshot(snapshot);
            QCoreApplication::processEvents();
            QVERIFY(diff->isVisible());
            QVERIFY(diff->viewport()->height() <= ChatPanel::kProposalDiffLines * line + 2 * diff->document()->documentMargin() + 1);
            QVERIFY(diff->viewport()->height() >= 2 * line);
            QVERIFY(diff->verticalScrollBar()->maximum() > 0);
            chat->proposalExpander()->click();
            QCoreApplication::processEvents();

            // The panel's corners are round: the stage shows past them.
            image = window.grab().toImage();
            const QPoint corner = chat->mapTo(&window, QPoint(1, 1));
            QCOMPARE(image.pixelColor(corner * ratio).rgb(), theme.viewerSurround.rgb());
            const QPoint inside = chat->mapTo(&window, QPoint(ChatPanel::kRadius, 2));
            QCOMPARE(image.pixelColor(inside * ratio).rgb(), theme.chatSurface.rgb());
            // The bar's Accept is the proposal's.
            bar->setValue(bar->maximum());
            QCoreApplication::processEvents();
            QVERIFY(sticky->isVisible());
            chat->barAcceptButton()->click();
            QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("accept"));
        }
    }

    void savedOwnKeysKeepTheOwnKeyConnection() {
        QSettings settings;
        const QVariant connection = settings.value(QStringLiteral("agent/connection"));
        const QVariant key = settings.value(QStringLiteral("agent/anthropic/apiKey"));
        settings.remove(QStringLiteral("agent/connection"));
        settings.setValue(QStringLiteral("agent/anthropic/apiKey"), QStringLiteral("sk-ant-saved"));
        {
            MainWindow window{QString(), QString()};
            QVERIFY(window.m_ownKeyRadio->isChecked());
        }
        settings.setValue(QStringLiteral("agent/anthropic/apiKey"), QString());
        settings.remove(QStringLiteral("agent/connection"));
        if (qEnvironmentVariableIsEmpty("ANTHROPIC_API_KEY") && qEnvironmentVariableIsEmpty("OPENAI_API_KEY")) {
            settings.setValue(QStringLiteral("agent/openai/apiKey"), QString());
            MainWindow window{QString(), QString()};
            QVERIFY(window.m_cloudRadio->isChecked());
        }
        settings.setValue(QStringLiteral("agent/connection"), QStringLiteral("cloud"));
        settings.setValue(QStringLiteral("agent/anthropic/apiKey"), QStringLiteral("sk-ant-saved"));
        {
            // An explicit choice wins over a saved key.
            MainWindow window{QString(), QString()};
            QVERIFY(window.m_cloudRadio->isChecked());
        }
        if (connection.isValid()) settings.setValue(QStringLiteral("agent/connection"), connection);
        else settings.remove(QStringLiteral("agent/connection"));
        settings.setValue(QStringLiteral("agent/anthropic/apiKey"), key);
    }

    void kerfCloudIsTheDefaultConnectionAndReflectsTheAccount() {
        MainWindow window{QString(), QString()};
        {
            const QSignalBlocker cloudBlock(window.m_cloudRadio);
            window.m_cloudRadio->setChecked(true);
        }
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(window.m_advancedSettings->isHidden());
        // Without a service, Kerf Cloud says so and points at Advanced.
        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "unavailable"}});
        QVERIFY(window.m_commands->reason(QStringLiteral("ai.ask")).contains(QStringLiteral("Settings → Advanced")));
        QVERIFY(window.m_cloudStatus->text().contains(QStringLiteral("isn't available yet")));
        QVERIFY(window.m_cloudSignIn->isHidden());
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("cloud")).toBool(), true);
        QVERIFY(!window.agentSettingsPayload().contains(QStringLiteral("apiKey")));

        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "signedOut"}});
        QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QStringLiteral("Sign in to Kerf Cloud in Settings"));
        QVERIFY(!window.m_cloudSignIn->isHidden());
        window.m_cloudToken->setText(QStringLiteral("tok-1"));
        window.m_cloudSignIn->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("cloud_sign_in"));
        QCOMPARE(window.m_lastCommand.value("token").toString(), QStringLiteral("tok-1"));

        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "ready"}, {"account", QJsonObject{
            {"email", "lim@example.com"}, {"plan", "Pro"}, {"ai", true}, {"credits", 1200}}}});
        QVERIFY(window.m_cloudStatus->text().contains(QStringLiteral("lim@example.com")));
        QVERIFY(window.m_cloudStatus->text().contains(QStringLiteral("1200 requests left")));
        QCOMPARE(window.m_statusAccount->text(), QStringLiteral("Kerf Cloud · Pro"));
        if (const QString shot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT"); !shot.isEmpty()) {
            window.m_preferences->show();
            QCoreApplication::processEvents();
            QVERIFY(window.m_preferences->grab().save(shot + QStringLiteral(".settings.png")));
            window.m_preferences->hide();
        }
        QVERIFY(window.m_cloudSignIn->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QString());
        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "outOfCredits"}, {"account", QJsonObject{
            {"email", "lim@example.com"}, {"plan", "Pro"}, {"ai", true}, {"credits", 0}}}});
        QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QStringLiteral("You've used this period's Kerf Cloud requests"));
        window.m_cloudSignOut->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("cloud_sign_out"));
        // A key of one's own needs the key.
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }
        window.m_anthropicActiveRadio->setChecked(true);
        window.m_anthropicKey->clear();
        window.updateCommandStates();
        if (qEnvironmentVariableIsEmpty("ANTHROPIC_API_KEY"))
            QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QStringLiteral("Add your API key in Settings → Advanced"));
    }

    void askingCutSendsTheSelectionAsFocus() {
        MainWindow window{QString(), QString()};
        window.show();
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"name", "Opening"}, {"timelineStart", 1.0}, {"duration", 2.0},
                            {"startFrame", 24}, {"durationFrames", 48}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }
        window.m_anthropicActiveRadio->setChecked(true);
        window.m_anthropicKey->setText(QStringLiteral("sk-ant-test"));
        window.updateCommandStates();
        window.m_commands->action(QStringLiteral("ai.ask"))->trigger();
        QTRY_VERIFY(window.m_askPrompt->isVisible());
        // Its field shows its focus as every field does: the ring around
        // its 5px bezel, and its edge in the accent.
        {
            QTextEdit *input = window.m_askPrompt->m_input;
            QTRY_VERIFY(input->hasFocus());
            QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
            const auto halos = window.m_askPrompt->findChildren<QWidget *>(QStringLiteral("focusHalo"));
            QCOMPARE(halos.size(), 1);
            QCOMPARE(halos.constFirst()->geometry(), input->geometry().adjusted(-3, -3, 3, 3));
            QCOMPARE(input->property("bezelRadius").toInt(), 5);
            const QImage prompt = window.m_askPrompt->grab().toImage();
            QCOMPARE(prompt.pixelColor(QPoint(input->x(), input->geometry().center().y()) * prompt.devicePixelRatio()).rgb(),
                     editorTheme().focusRing.rgb());
        }
        QVERIFY(window.m_askPrompt->m_title->text().contains(QStringLiteral("Opening")));
        QCOMPARE(window.m_askPrompt->scope(), QStringLiteral("clip"));
        window.m_askPrompt->m_input->setPlainText(QStringLiteral("Tighten the start"));
        if (const QString shot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT"); !shot.isEmpty())
            QVERIFY(window.m_askPrompt->grab().save(shot + QStringLiteral(".ask.png")));
        QTest::keyClick(window.m_askPrompt->m_input, Qt::Key_Return);
        QVERIFY(!window.m_askPrompt->isVisible());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value("text").toString(), QStringLiteral("Tighten the start"));
        const QJsonObject focus = window.m_lastCommand.value("focus").toObject();
        QCOMPARE(focus.value("scope").toString(), QStringLiteral("clip"));
        QCOMPARE(focus.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(focus.value("fromFrame").toInt(), 24);
        QCOMPARE(focus.value("toFrame").toInt(), 72);
        // While Kerf works with its conversation closed, the timeline says
        // so and offers Stop.
        window.setChatOpen(false);
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QVERIFY(window.m_aiBar->isVisible());
        QVERIFY(window.m_aiBarText->text().contains(QStringLiteral("Opening")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("ai.stop")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QStringLiteral("Kerf is working on a request"));
    }

    void proposalShowsTheScriptAndDocumentDiff() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject proposal{{"status", "pending"}, {"proposalId", "proposal-1"},
                             {"script", "trim c1 out -24f\n"},
                             {"documentDiff", "--- before\n+++ after\n@@ -1,1 +1,1 @@\n-    clip c1 dur=00:00:04:00\n+    clip c1 dur=00:00:03:00\n"}};
        window.updateProposal(proposal);
        const auto text = window.m_proposalDiff->toPlainText();
        QVERIFY(text.contains(QStringLiteral("trim c1 out -24f")));
        QVERIFY(text.contains(QStringLiteral("+    clip c1 dur=00:00:03:00")));
        // Counted from its script's changes, as its rows list them (a narrow
        // panel leaves the count to the rows; the whole summary is its tip).
        QCOMPARE(window.m_chat->proposalBarSummary()->toolTip(), QStringLiteral("Trim · 1 change"));
        QVERIFY(window.m_proposalSummary->text() == QStringLiteral("Trim · 1 change") ||
                (window.m_chat->width() < ChatPanel::kNarrowProposal && window.m_proposalSummary->text() == QStringLiteral("Trim")));
        QCOMPARE(window.m_chat->proposalChangeRows().size(), 1);
        QCOMPARE(window.m_chat->proposalChangeRows().constFirst()->change().description, QStringLiteral("Trim c1"));
        QCOMPARE(window.m_chat->proposalChangeRows().constFirst()->change().detail, QStringLiteral("end \u22121 s"));
        QVERIFY(window.m_acceptButton->isEnabled());
        proposal.insert(QStringLiteral("freshness"), QStringLiteral("stale"));
        window.updateProposal(proposal);
        QVERIFY(!window.m_acceptButton->isEnabled());
        QVERIFY(window.m_proposalDiff->toPlainText().contains(QStringLiteral("out of date")));
    }

    // The conversation scrolled to its latest, as a comments panel rests:
    // the latest comment's foot exactly 16 points over the chips or the
    // composer; at the top the rows are cut cleanly at the view's edge, right
    // under the header (or the compact proposal bar) and its hairline, with
    // no cover, fade or blank band; the hairline shows while the list is
    // scrolled away from its top.
    // The list scrolls freely and its edges cut rows cleanly: where a cut
    // would leave only a sliver of a line (its ascenders over the bottom
    // edge, its descenders under the top) or of a symbol, the panel's
    // surface covers it.
    // A file name in a comment wraps as one word: never "fisher-" at the
    // end of a line and "close.mov." on the next.
    void chatBodyKeepsFileNamesWhole() {
        const QString text = QStringLiteral("I raised the exposure and the temperature on fisher-close.mov.");
        ChatPanel chat;
        CommentRow *row = chat.addComment(CommentRow::Kind::Kerf, text);
        chat.resize(ViewerWidget::kDefaultChatWidth, 520);
        chat.show();
        QVERIFY(QTest::qWaitForWindowExposed(&chat));
        QLabel *body = row->body();
        body->ensurePolished();
        // What the row copies is the text as written.
        QCOMPARE(row->text(), text);
        const auto linesAt = [body](const QString &html, int width) {
            QTextDocument document;
            document.setDocumentMargin(0);
            document.setDefaultFont(body->font());
            document.setHtml(html);
            document.setTextWidth(width);
            document.size(); // laid out
            QStringList lines;
            for (QTextBlock block = document.begin(); block.isValid(); block = block.next())
                for (int index = 0; index < block.layout()->lineCount(); ++index) {
                    const QTextLine line = block.layout()->lineAt(index);
                    lines << block.text().mid(line.textStart(), line.textLength()).trimmed();
                }
            return lines;
        };
        int cases = 0;
        // From the narrowest width the name fits on a line by itself to the
        // row's own.
        const int name = int(std::ceil(QFontMetricsF(body->font()).horizontalAdvance(QStringLiteral("fisher-close.mov.")))) + 2;
        for (int width = name; width <= body->contentsRect().width(); ++width) {
            // Only the widths where the text as written would break at the hyphen.
            const QStringList plain = linesAt(text.toHtmlEscaped(), width);
            if (std::none_of(plain.begin(), plain.end(), [](const QString &line) { return line.endsWith(QLatin1Char('-')); })) continue;
            ++cases;
            const QStringList shown = linesAt(body->text(), width);
            for (const QString &line : shown)
                QVERIFY2(!line.endsWith(QLatin1Char('-')), qPrintable(QStringLiteral("at %1: “%2”").arg(width).arg(line)));
            QVERIFY2(std::any_of(shown.begin(), shown.end(), [](const QString &line) { return line.contains(QStringLiteral("close.mov")) && line.contains(QStringLiteral("fisher")); }),
                     qPrintable(QStringLiteral("at %1: %2").arg(width).arg(shown.join(QStringLiteral(" | ")))));
        }
        QVERIFY2(cases > 0, "no width breaks the name as written");
        // Selected and copied with ⌘C, the text lands on the clipboard as
        // written, without the invisible break hints.
        QApplication::clipboard()->clear();
        QTextDocument shown;
        shown.setHtml(body->text());
        body->setSelection(0, int(shown.characterCount()) - 1);
        QVERIFY(body->hasSelectedText());
        QVERIFY(body->selectedText().contains(QChar(0x2060)));
        QKeyEvent override(QEvent::ShortcutOverride, Qt::Key_C, Qt::ControlModifier);
        QCoreApplication::sendEvent(body, &override);
        QVERIFY(override.isAccepted());
        QKeyEvent copy(QEvent::KeyPress, Qt::Key_C, Qt::ControlModifier);
        QCoreApplication::sendEvent(body, &copy);
        QCOMPARE(QApplication::clipboard()->text(), text);
    }

    void chatEdgesLeaveNoSlivers() {
        ChatPanel chat;
        for (int index = 0; index < 14; ++index)
            chat.addComment(index % 2 ? CommentRow::Kind::Kerf : CommentRow::Kind::User,
                            QStringLiteral("Comment %1: tighten the cut after the drone pass and warm the dusk shots a little.").arg(index));
        chat.resize(300, 520);
        chat.show();
        QVERIFY(QTest::qWaitForWindowExposed(&chat));
        QScrollBar *bar = chat.scroll()->verticalScrollBar();
        QTRY_VERIFY(bar->maximum() > 200);
        QWidget *viewport = chat.scroll()->viewport();
        int covered = 0;
        for (int value = 0; value <= bar->maximum(); value += 3) {
            bar->setValue(value);
            QCoreApplication::processEvents();
            const QList<QRect> strips = chat.edgeCoverStrips();
            QRegion cover;
            for (const QRect &strip : strips) cover += strip;
            for (QLabel *label : chat.scroll()->widget()->findChildren<QLabel *>()) {
                if (!label->isVisibleTo(chat.scroll()->widget()) || label->text().isEmpty()) continue;
                const QList<TextLine> lines = labelTextLines(label);
                QVERIFY(!lines.isEmpty());
                // The lines lie inside the label, their capitals' tops in them.
                QVERIFY2(label->rect().adjusted(0, -1, 0, 1).contains(lines.constFirst().rect) &&
                             label->rect().adjusted(0, -1, 0, 1).contains(lines.constLast().rect),
                         qPrintable(label->text().left(30)));
                for (const TextLine &text : lines) {
                    QVERIFY(text.capTop > text.rect.top() && text.capTop < text.rect.top() + text.rect.height() / 2);
                    const QRect line = text.rect.translated(label->mapTo(viewport, QPoint()));
                    const int capTop = text.capTop + label->mapTo(viewport, QPoint()).y();
                    // Under the top edge: a line whose baseline shows shows
                    // its capitals' tops too, or none of it.
                    if (line.top() < 0 && line.bottom() >= 0) {
                        const QRegion shownPart = QRegion(QRect(line.left(), 0, line.width(), line.bottom() + 1)).subtracted(cover);
                        if (capTop < 0)
                            QVERIFY2(shownPart.isEmpty(), qPrintable(QStringLiteral("at %1: a line of “%2” cut through its capitals").arg(value).arg(label->text().left(20))));
                        else
                            QVERIFY2(shownPart == QRegion(QRect(line.left(), 0, line.width(), line.bottom() + 1)),
                                     qPrintable(QStringLiteral("at %1: a line of “%2” with its capitals covered").arg(value).arg(label->text().left(20))));
                    }
                    const int shown = viewport->height() - line.top();
                    if (shown > 0 && shown < ChatPanel::kSliverLine * line.height())
                        QVERIFY2(QRegion(QRect(line.left(), line.top(), line.width(), shown)).subtracted(cover).isEmpty(),
                                 qPrintable(QStringLiteral("at %1: %2 px of a line of “%3” left").arg(value).arg(shown).arg(label->text().left(20))));
                    const int over = line.bottom() + 1;
                    if (line.top() < 0 && over > 0 && over < ChatPanel::kSliverLine * line.height())
                        QVERIFY2(QRegion(QRect(line.left(), 0, line.width(), over)).subtracted(cover).isEmpty(),
                                 qPrintable(QStringLiteral("at %1: %2 px of a line's foot left").arg(value).arg(over)));
                }
            }
            // A text button's words ("Copy", "Ask again", "2 steps") by the
            // same rule as a label's line: under the top edge, shown with
            // their capitals' tops or not at all.
            int buttons = 0;
            for (QAbstractButton *button : chat.scroll()->widget()->findChildren<QAbstractButton *>()) {
                if (!button->isVisibleTo(chat.scroll()->widget()) || button->text().isEmpty() || !button->findChildren<QWidget *>().isEmpty())
                    continue;
                ++buttons;
                const TextLine text = buttonTextLine(button);
                QVERIFY2(button->rect().adjusted(0, -1, 0, 1).contains(text.rect), qPrintable(button->text()));
                QVERIFY(text.capTop > text.rect.top() && text.capTop < text.rect.top() + text.rect.height() / 2);
                const QPoint origin = button->mapTo(viewport, QPoint());
                const QRect line = text.rect.translated(origin);
                const int capTop = text.capTop + origin.y();
                if (line.top() < 0 && line.bottom() >= 0) {
                    const QRect inView(line.left(), 0, line.width(), line.bottom() + 1);
                    const QRegion shownPart = QRegion(inView).subtracted(cover);
                    if (capTop < 0)
                        QVERIFY2(shownPart.isEmpty(), qPrintable(QStringLiteral("at %1: “%2” cut through its capitals").arg(value).arg(button->text())));
                    else
                        QVERIFY2(shownPart == QRegion(inView), qPrintable(QStringLiteral("at %1: “%2” with its capitals covered").arg(value).arg(button->text())));
                }
            }
            QVERIFY(buttons > 0);
            // An avatar's arc under the top edge or over the bottom one.
            for (ChatAvatar *avatar : chat.scroll()->widget()->findChildren<ChatAvatar *>()) {
                if (!avatar->isVisibleTo(chat.scroll()->widget())) continue;
                const QRect box(avatar->mapTo(viewport, QPoint()), avatar->size());
                const double least = std::max<double>(ChatPanel::kSliver, ChatPanel::kSliverAvatar * box.height());
                if (box.top() < 0 && box.bottom() >= 0 && box.bottom() + 1 < least)
                    QVERIFY2(QRegion(QRect(box.left(), 0, box.width(), box.bottom() + 1)).subtracted(cover).isEmpty(), qPrintable(QString::number(value)));
                const int shown = viewport->height() - box.top();
                if (shown > 0 && box.bottom() >= viewport->height() && shown < least)
                    QVERIFY2(QRegion(QRect(box.left(), box.top(), box.width(), shown)).subtracted(cover).isEmpty(), qPrintable(QString::number(value)));
            }
            // A line half in view or more over the bottom edge is never
            // covered; under the top edge, no strip is taller than a line.
            for (const QRect &strip : strips) {
                if (strip.top() > 0) QVERIFY(strip.height() <= std::max(ChatPanel::kSliverAvatar * ChatPanel::kAvatar, 0.5 * 24));
                else QVERIFY(strip.height() < 24);
            }
            if (!strips.isEmpty() && covered < 3) {
                // The cover is the panel's own surface, solid.
                ++covered;
                const QImage image = chat.grab().toImage();
                const QRect strip = strips.first().translated(viewport->mapTo(&chat, QPoint()));
                if (strip.width() > 4 && strip.height() >= 2)
                    QCOMPARE(image.pixelColor(QPoint(strip.center().x(), strip.top()) * image.devicePixelRatio()).rgb(), editorTheme().chatSurface.rgb());
            }
        }
        QVERIFY2(covered > 0, "some scroll position cuts a line to a sliver");
    }

    // The conversation's states in the 9:16 stage, in both appearances:
    // the welcome before the first message, a long conversation with Kerf
    // streaming its reply, a pending proposal pinned at the top of the list
    // with its Accept and Reject pills, and, scrolled away under a longer
    // conversation, the compact bar that keeps them in reach. With
    // KERF_SCREENSHOT_DIR set, each
    // is saved as editor-9x16-<state>-<scheme>.png.
    void conversationRendersItsStates() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
        });
        {
            const EditorTheme &theme = editorTheme();
            const QString schemeName = QStringLiteral("dark");
            const auto open = [&](MainWindow &window, bool proposal) {
                QSettings().remove(QStringLiteral("workspace"));
                QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
                QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
                timeline.insert(QStringLiteral("width"), 1080);
                timeline.insert(QStringLiteral("height"), 1920);
                snapshot.insert(QStringLiteral("timeline"), timeline);
                if (!proposal) {
                    snapshot.remove(QStringLiteral("pendingProposal"));
                    snapshot.remove(QStringLiteral("proposalTimeline"));
                }
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshot);
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                window.m_viewer->setFrame(testing::footageFrame(testing::Shot::FisherClose, QSize(270, 480)), 0.0);
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
            };
            QImage image;
            const auto save = [&](MainWindow &window, const QString &state) {
                QTest::qWait(30);
                image = window.grab().toImage();
                QCOMPARE(amberPixels(image), 0);
                if (!shots.isEmpty())
                    QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("editor-9x16-%1-%2.png").arg(state, schemeName))));
            };
            {
                // The welcome: Kerf's first comment, and two requests to try.
                MainWindow window{QString(), QString()};
                open(window, false);
                QVERIFY(window.m_chat->welcome()->isVisible());
                QVERIFY(window.m_chat->countLabel()->text().isEmpty());
                QCOMPARE(window.m_rail->chatButton()->label(), QStringLiteral("Chat"));
                // Plain rows of words after an arrow, no chip: at least 32
                // points, on at most two lines.
                for (QAbstractButton *example : window.m_chat->examples()) {
                    QVERIFY(example->isVisible());
                    QVERIFY(qobject_cast<SuggestionButton *>(example));
                    QVERIFY(example->height() >= 32 && example->height() <= 2 * 20 + 12);
                }
                QVERIFY(!window.m_chat->statusRow()->isVisible());
                save(window, QStringLiteral("welcome"));
                // At the panel's narrowest an example wraps onto a second line
                // rather than end in an ellipsis, and never runs past the
                // panel's padding.
                window.m_viewer->setChatWidth(ViewerWidget::kMinChatWidth);
                QCoreApplication::processEvents();
                QCOMPARE(window.m_chat->width(), ViewerWidget::kMinChatWidth);
                for (QAbstractButton *example : window.m_chat->examples()) {
                    QVERIFY2(example->mapTo(window.m_chat, QPoint(example->width(), 0)).x() <= window.m_chat->width() - ChatPanel::kSidePadding,
                             qPrintable(example->text()));
                    auto *suggestion = static_cast<SuggestionButton *>(example);
                    QFont font = example->font();
                    font.setPixelSize(13);
                    // Lines counted as the button lays them out: a wrapped
                    // block's pixel height rounds up past twice a rounded
                    // line spacing where the font's line height is fractional
                    // (DejaVu Sans on Linux).
                    QTextLayout layout(example->text(), font);
                    QTextOption option;
                    option.setWrapMode(QTextOption::WordWrap);
                    layout.setTextOption(option);
                    layout.beginLayout();
                    int lines = 0;
                    for (QTextLine line = layout.createLine(); line.isValid(); line = layout.createLine()) {
                        line.setLineWidth(suggestion->textRect().width());
                        ++lines;
                    }
                    layout.endLayout();
                    QVERIFY2(lines <= 2, qPrintable(QStringLiteral("%1: %2 lines").arg(example->text()).arg(lines)));
                }
                QVERIFY(window.m_chatScroll->widget()->width() <= window.m_chatScroll->viewport()->width());
                window.m_viewer->setChatWidth(ViewerWidget::kDefaultChatWidth);
            }
            {
                // A long conversation, Kerf streaming its latest reply: its
                // three dots and its latest step after the latest comment,
                // and Stop for Send.
                MainWindow window{QString(), QString()};
                open(window, false);
                const QStringList asks{QStringLiteral("Open on the harbor wide, and hold it for four seconds."),
                                       QStringLiteral("Cut to the fisher close-up on the first beat."),
                                       QStringLiteral("Can the drone pass come in under the title?"),
                                       QStringLiteral("Lower the music under the dialogue.")};
                const QStringList answers{QStringLiteral("Done: the wide holds for 4 s and the close-up starts on the downbeat at 00:00:04:00."),
                                          QStringLiteral("The close-up now cuts in on the first beat."),
                                          QStringLiteral("I moved the drone pass to V2 under the title, with a 12-frame dissolve.")};
                for (int turn = 0; turn < asks.size(); ++turn) {
                    window.appendConversation(QStringLiteral("You"), asks.at(turn));
                    if (turn < answers.size()) {
                        window.appendConversation(QStringLiteral("Kerf"), answers.at(turn));
                        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Edited the timeline"));
                    }
                }
                window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
                window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "Reading the music's loudness"}});
                window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "I’ll duck the score by 8 dB "}});
                window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "wherever someone speaks, with"}});
                QCoreApplication::processEvents();
                const QList<CommentRow *> rows = window.m_chat->rows();
                QCOMPARE(rows.size(), 8);
                QCOMPARE(rows.constLast()->text(), QStringLiteral("I’ll duck the score by 8 dB wherever someone speaks, with"));
                QCOMPARE(rows.constLast()->replies(), QStringList{QStringLiteral("Reading the music's loudness")});
                QVERIFY(window.m_chat->statusRow()->isVisible());
                QVERIFY(window.m_chatActivity->isVisible());
                QCOMPARE(window.m_chatActivity->text(), QStringLiteral("Reading the music's loudness"));
                QVERIFY(window.m_abortButton->isVisible() && !window.m_sendButton->isVisible());
                // It followed the stream to the latest.
                QScrollBar *bar = window.m_chatScroll->verticalScrollBar();
                QTRY_VERIFY(bar->maximum() > 0);
                QTRY_COMPARE(bar->value(), bar->maximum());
                verifyListEnd(window.m_chat);
                save(window, QStringLiteral("streaming"));
            }
            {
                // A pending proposal, pinned under the header with its changes
                // listed as rows: Accept filled with the accent, Reject tonal,
                // both 36px pills. It arrives after Kerf's reply, as it does,
                // and the list shows it from its head.
                MainWindow window{QString(), QString()};
                open(window, false);
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
                window.appendConversation(QStringLiteral("Kerf"),
                    QStringLiteral("Here’s the cut: the close-up is 1.5 s shorter and the drone pass is gone. Review it before it changes the cut."));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b by 36 frames"));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Removed v1c"));
                {
                    QJsonObject snapshot = showcaseSnapshot(13, QStringLiteral("clean"));
                    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
                    timeline.insert(QStringLiteral("width"), 1080);
                    timeline.insert(QStringLiteral("height"), 1920);
                    snapshot.insert(QStringLiteral("timeline"), timeline);
                    window.applySnapshot(snapshot);
                }
                QCoreApplication::processEvents();
                QVERIFY(window.m_proposalBox->isVisible());
                // Pinned at the top of the list, 16 points under its top.
                QCOMPARE(window.m_proposalBox->parentWidget(), window.m_chatScroll->widget());
                QCOMPARE(window.m_proposalBox->y(), 16);
                QVERIFY(window.m_proposalBox->findChild<QLabel *>(QStringLiteral("chatPinned"))->text().startsWith(QStringLiteral("Pinned")));
                // Under "Pinned by Kerf", the name line every comment has, then its text.
                QCOMPARE(window.m_chat->proposalName()->text(), QStringLiteral("Kerf"));
                QCOMPARE(window.m_chat->proposalTime()->text(), QStringLiteral("now"));
                QVERIFY(window.m_chat->proposalName()->isVisible() && window.m_chat->proposalTime()->isVisible());
                QVERIFY(window.m_chat->proposalName()->mapTo(window.m_chat, QPoint()).y() <
                        window.m_chat->proposalSummary()->mapTo(window.m_chat, QPoint()).y());
                for (QPushButton *pill : {window.m_acceptButton, window.m_rejectButton}) {
                    QVERIFY(pill->isVisible() && pill->isEnabled());
                    QCOMPARE(pill->height(), ChatPanel::kPillHeight);
                }
                QVERIFY(window.m_rejectButton->geometry().right() < window.m_acceptButton->geometry().left());
                QCOMPARE(ChatPanel::kPillHeight, 36);
                // The summary counts the rows it heads, in words.
                QCOMPARE(window.m_proposalSummary->text(), QStringLiteral("Trim, remove, add a title · 3 changes"));
                QWidget *changes = window.m_chat->proposalChanges();
                QVERIFY(changes->isVisible());
                // Each change a row: a glyph, what it does in words, and a
                // detail in secondary ink at the right. No script.
                const QList<ProposalChangeRow *> rows = window.m_chat->proposalChangeRows();
                QCOMPARE(rows.size(), 3);
                QTRY_VERIFY(changes->height() >= 3 * ProposalChangeRow::kHeight);
                const QList<QPair<QString, QString>> expected{{QStringLiteral("Trim fisher-close"), QStringLiteral("end \u22121.5 s")},
                                                              {QStringLiteral("Remove drone-pass"), QStringLiteral("0:11\u2009\u2013\u20090:18")},
                                                              {QStringLiteral("Add title “Lower third”"), QStringLiteral("at 0:12")}};
                const QList<ProposalChange::Glyph> glyphs{ProposalChange::Glyph::Trim, ProposalChange::Glyph::Remove, ProposalChange::Glyph::Title};
                for (int index = 0; index < rows.size(); ++index) {
                    ProposalChangeRow *row = rows.at(index);
                    QCOMPARE(row->change().description, expected.at(index).first);
                    QCOMPARE(row->change().detail, expected.at(index).second);
                    QCOMPARE(row->change().glyph, glyphs.at(index));
                    QCOMPARE(row->height(), ProposalChangeRow::kHeight);
                    // The glyph at the text column, the detail at its right end.
                    QCOMPARE(row->mapTo(window.m_chat, row->glyphRect().topLeft()).x(),
                             window.m_chat->proposalSummary()->mapTo(window.m_chat, QPoint()).x());
                    QCOMPARE(row->detailRect().right(), row->width() - 1);
                    QVERIFY(row->descriptionRect().right() < row->detailRect().left());
                    if (index) QCOMPARE(row->y(), rows.at(index - 1)->y() + ProposalChangeRow::kHeight);
                }
                // The pills under the rows, then a hairline closing the pinned
                // comment 16 points under them.
                QVERIFY(window.m_acceptButton->mapTo(window.m_proposalBox, QPoint()).y() >
                        rows.constLast()->mapTo(window.m_proposalBox, QPoint(0, rows.constLast()->height())).y() - 1);
                QFrame *rule = window.m_chat->proposalRule();
                QVERIFY(rule->isVisible());
                QCOMPARE(rule->height(), 1);
                QCOMPARE(rule->mapTo(window.m_proposalBox, QPoint()).y(),
                         window.m_acceptButton->mapTo(window.m_proposalBox, QPoint(0, window.m_acceptButton->height())).y() + 16);
                QCOMPARE(rule->width(), window.m_proposalBox->width());
                // Opening the script folds the rows; the script has every line.
                window.m_chat->proposalExpander()->click();
                QCoreApplication::processEvents();
                QVERIFY(!changes->isVisible());
                QVERIFY(window.m_proposalDiff->toPlainText().contains(QStringLiteral("trim v1b out -36f")));
                window.m_chat->proposalExpander()->click();
                QCoreApplication::processEvents();
                QVERIFY(changes->isVisible());
                // Wide, "Script" ends the summary's line; in a narrow panel (as
                // beside a laptop) it starts the pills' row instead, so the
                // summary keeps one line, and each change keeps its words
                // whole, its detail giving way first.
                {
                    ChatExpander *script = window.m_chat->proposalExpander();
                    const auto middle = [&](QWidget *widget) { return widget->mapTo(window.m_chat, QPoint(0, widget->height() / 2)).y(); };
                    QVERIFY(window.m_chat->width() >= ChatPanel::kNarrowProposal);
                    QVERIFY(std::abs(middle(script) - middle(window.m_proposalSummary)) <= window.m_proposalSummary->height() / 2);
                    window.m_viewer->setChatWidth(ViewerWidget::kMinChatWidth);
                    QCoreApplication::processEvents();
                    QCOMPARE(window.m_chat->width(), ViewerWidget::kMinChatWidth);
                    QVERIFY(std::abs(middle(script) - middle(window.m_acceptButton)) <= 1);
                    QCOMPARE(script->mapTo(window.m_chat, QPoint()).x(), window.m_proposalSummary->mapTo(window.m_chat, QPoint()).x());
                    QVERIFY(script->mapTo(window.m_chat, QPoint(script->width(), 0)).x() < window.m_rejectButton->mapTo(window.m_chat, QPoint()).x());
                    QCOMPARE(window.m_proposalSummary->height(), window.m_proposalSummary->heightForWidth(window.m_proposalSummary->width()));
                    QVERIFY2(window.m_proposalSummary->heightForWidth(window.m_proposalSummary->width()) < 2 * window.m_proposalSummary->fontMetrics().lineSpacing(),
                             "the summary wraps beside a narrow panel");
                    // Its count is left to the rows under it (and the tooltip).
                    QCOMPARE(window.m_proposalSummary->text(), QStringLiteral("Trim, remove, add a title"));
                    QCOMPARE(window.m_proposalSummary->toolTip(), QStringLiteral("Trim, remove, add a title · 3 changes"));
                    // "Script" whole on the row; the pills unclipped, the row
                    // inside the column.
                    QVERIFY(script->isCompact() && script->labelShown());
                    QVERIFY(window.m_acceptButton->mapTo(window.m_chat, QPoint(window.m_acceptButton->width(), 0)).x() <=
                            window.m_chatScroll->viewport()->mapTo(window.m_chat, QPoint(window.m_chatScroll->viewport()->width(), 0)).x() - ChatPanel::kSidePadding);
                    QCOMPARE(window.m_chatScroll->widget()->width(), window.m_chatScroll->viewport()->width());
                    // The detail (an amount, a time) is never cut: the words
                    // drop the clip's extension first, then the detail takes a
                    // second line under them. Only the words could be elided,
                    // and here none is; the tooltip says it whole.
                    const QList<ProposalChangeRow *> narrow = window.m_chat->proposalChangeRows();
                    const QStringList shortWords{QStringLiteral("Trim fisher-close"), QStringLiteral("Remove drone-pass"), QStringLiteral("Add title “Lower third”")};
                    for (int index = 0; index < narrow.size(); ++index) {
                        ProposalChangeRow *row = narrow.at(index);
                        QFont detail = row->font();
                        detail.setPixelSize(12);
                        detail.setFeature(QFont::Tag("tnum"), 1);
                        const int detailWide = QFontMetrics(detail).horizontalAdvance(row->change().detail);
                        QCOMPARE(row->change().shortDescription, shortWords.at(index));
                        QCOMPARE(row->shownDetail(), row->change().detail);
                        QVERIFY2(row->detailRect().width() >= detailWide, qPrintable(row->change().detail));
                        QVERIFY2(!row->shownDescription().endsWith(QChar(0x2026)), qPrintable(row->shownDescription()));
                        QVERIFY(row->shownDescription() == row->change().description || row->shownDescription() == row->change().shortDescription);
                        if (row->shownDescription() != row->change().description)
                            QCOMPARE(row->toolTip(), row->change().description + QStringLiteral(" · ") + row->change().detail);
                        QCOMPARE(row->detailRect().right(), row->width() - 1);
                        QVERIFY(row->descriptionRect().left() == ProposalChangeRow::kTextLeft);
                        if (row->twoLines()) {
                            // The detail under the words, at their x; the row
                            // grows to hold both lines.
                            QCOMPARE(row->height(), ProposalChangeRow::kTwoLineHeight);
                            QCOMPARE(row->detailRect().left(), ProposalChangeRow::kTextLeft);
                            QVERIFY(row->detailRect().top() >= row->descriptionRect().bottom());
                            QVERIFY(row->detailRect().bottom() < row->height());
                        } else {
                            QCOMPARE(row->height(), ProposalChangeRow::kHeight);
                            QVERIFY(row->descriptionRect().right() < row->detailRect().left());
                        }
                        if (index) QCOMPARE(row->y(), narrow.at(index - 1)->y() + narrow.at(index - 1)->height());
                    }
                    // A row narrower still keeps its detail whole on its own line.
                    {
                        ProposalChange change = narrow.at(1)->change();
                        ProposalChangeRow tight(change);
                        tight.resize(170, ProposalChangeRow::kHeight);
                        tight.show();
                        QCoreApplication::processEvents();
                        QCOMPARE(tight.width(), 170);
                        QVERIFY(tight.twoLines());
                        QCOMPARE(tight.height(), ProposalChangeRow::kTwoLineHeight);
                        QCOMPARE(tight.shownDetail(), change.detail);
                        QVERIFY(tight.shownDescription() == change.description || tight.shownDescription() == change.shortDescription);
                        if (tight.shownDescription() != change.description)
                            QCOMPARE(tight.toolTip(), change.description + QStringLiteral(" · ") + change.detail);
                    }
                    window.m_viewer->setChatWidth(ViewerWidget::kDefaultChatWidth);
                    QCoreApplication::processEvents();
                    QVERIFY(std::abs(middle(script) - middle(window.m_proposalSummary)) <= window.m_proposalSummary->height() / 2);
                    QVERIFY(!script->isCompact());
                    QCOMPARE(window.m_proposalSummary->text(), QStringLiteral("Trim, remove, add a title · 3 changes"));
                }
                // Shown from its head: 16 points under the header, no hairline
                // and no compact bar.
                QCOMPARE(window.m_chatScroll->verticalScrollBar()->value(), 0);
                QCOMPARE(window.m_proposalBox->mapTo(window.m_chat, QPoint()).y(), ChatPanel::kHeaderHeight + 1 + 16);
                QVERIFY(!window.m_chat->listEdge()->property("scrolled").toBool());
                QVERIFY(!window.m_chat->proposalBar()->isVisible());
                save(window, QStringLiteral("proposal"));
                const qreal ratio = image.devicePixelRatio();
                const auto fill = [&](QPushButton *pill) {
                    return image.pixelColor(pill->mapTo(&window, QPoint(pill->height() / 2 - 6, pill->height() / 2)) * ratio).rgb();
                };
                QCOMPARE(fill(window.m_acceptButton), theme.accent.rgb());
                QVERIFY(nearColor(fill(window.m_rejectButton), laidOn(theme.chatSurface, theme.pillTonal).rgb()));
                QVERIFY(contrastRatio(theme.onAccent, theme.accent) >= 4.5);
                // A longer conversation scrolls the proposal away: the compact
                // bar under the header keeps its summary and pills, its
                // Accept filled the same, and the hairline under it.
                window.m_chatScroll->verticalScrollBar()->setValue(window.m_chatScroll->verticalScrollBar()->maximum());
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Before I accept: does the lower third still land on the beat?"));
                window.appendConversation(QStringLiteral("Kerf"),
                    QStringLiteral("Yes. It starts at 00:00:12:00, on the downbeat after the close-up, and holds for three seconds."));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("And the music under it?"));
                window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("Unchanged: the score keeps its level until the dialogue starts."));
                QTRY_VERIFY(window.m_chat->proposalBar()->isVisible());
                verifyListEnd(window.m_chat);
                save(window, QStringLiteral("proposal-scrolled"));
                QCOMPARE(fill(window.m_chat->barAcceptButton()), theme.accent.rgb());
                window.m_chatScroll->verticalScrollBar()->setValue(0);
                QCoreApplication::processEvents();
                QVERIFY(!window.m_chat->proposalBar()->isVisible());
                // Accept and Reject run the proposal's commands.
                window.m_acceptButton->click();
                QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("accept"));
            }
            {
                // 16:9, the laptop's shorter panel, a short conversation: the
                // proposal arrives and the list shows it from its head, so its
                // "Pinned by Kerf" line, avatar and summary are in view, not
                // only its pills. Scrolled to the latest, its head scrolls away
                // and the compact bar keeps the decision in reach.
                QSettings().remove(QStringLiteral("workspace"));
                MainWindow window{QString(), QString()};
                QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
                snapshot.remove(QStringLiteral("pendingProposal"));
                snapshot.remove(QStringLiteral("proposalTimeline"));
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshot);
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                window.m_viewer->setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(480, 270)), 0.0);
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
                window.appendConversation(QStringLiteral("Kerf"),
                    QStringLiteral("Here’s the cut: the close-up is 1.5 s shorter and the drone pass is gone. Review it before it changes the cut."));
                QCoreApplication::processEvents();
                window.applySnapshot(showcaseSnapshot(13, QStringLiteral("clean")));
                QTest::qWait(30);
                QCOMPARE(window.m_viewer->device(), ViewerWidget::Device::Laptop);
                QScrollBar *bar = window.m_chatScroll->verticalScrollBar();
                QVERIFY2(bar->maximum() > 0, "the conversation is taller than the laptop's panel");
                QCOMPARE(bar->value(), 0);
                QWidget *pin = window.m_proposalBox->findChild<QLabel *>(QStringLiteral("chatPinned"));
                const int viewportTop = window.m_chatScroll->viewport()->mapTo(window.m_chat, QPoint()).y();
                QVERIFY(pin->mapTo(window.m_chat, QPoint()).y() >= viewportTop);
                QVERIFY(window.m_chat->proposalSummary()->visibleRegion().boundingRect().height() > 0);
                QVERIFY(!window.m_chat->proposalBar()->isVisible());
                // The compact bar says what the proposal does where that fits
                // whole, and only the count, whole, where it does not: at
                // every width, never a summary that ends in an ellipsis.
                bar->setValue(bar->maximum());
                QCoreApplication::processEvents();
                QTRY_VERIFY(window.m_chat->proposalBar()->isVisible());
                for (const int panel : {ViewerWidget::kMinChatWidth, 331, ViewerWidget::kDefaultChatWidth}) {
                    window.m_viewer->setChatWidth(panel);
                    bar->setValue(bar->maximum());
                    QCoreApplication::processEvents();
                    QTRY_VERIFY(window.m_chat->proposalBar()->isVisible());
                    QLabel *summary = window.m_chat->proposalBarSummary();
                    QVERIFY2(!summary->text().endsWith(QChar(0x2026)), qPrintable(QStringLiteral("%1: %2").arg(panel).arg(summary->text())));
                    QVERIFY2(summary->text() == QStringLiteral("Trim, remove, add a title") || summary->text() == QStringLiteral("3 changes"),
                             qPrintable(summary->text()));
                    QVERIFY(QFontMetrics(summary->font()).horizontalAdvance(summary->text()) <= summary->contentsRect().width());
                }
                window.m_viewer->setChatWidth(ViewerWidget::kMinChatWidth);
                bar->setValue(bar->maximum());
                QCoreApplication::processEvents();
                QTRY_VERIFY(window.m_chat->proposalBar()->isVisible());
                QCOMPARE(window.m_chat->proposalBarSummary()->text(), QStringLiteral("3 changes"));
                bar->setValue(0);
                QCoreApplication::processEvents();
                QVERIFY(!window.m_chat->proposalBar()->isVisible());
                window.m_viewer->setChatWidth(ViewerWidget::kDefaultChatWidth);
                QTest::qWait(30);
                const QImage wide = window.grab().toImage();
                QCOMPARE(amberPixels(wide), 0);
                if (!shots.isEmpty())
                    QVERIFY(wide.save(QDir(shots).filePath(QStringLiteral("editor-16x9-proposal-%1.png").arg(schemeName))));
            }
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::CommentsTest)

#include "comments_test.moc"
