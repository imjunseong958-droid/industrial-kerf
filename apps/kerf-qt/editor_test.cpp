#include "ask_prompt.h"
#include "command_registry.h"
#include "editor_common.h"
#include "effect_controls_panel.h"
#include "effects_panel.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "workspace.h"

#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QDialogButtonBox>
#include <QDoubleSpinBox>
#include <QFile>
#include <QFileInfo>
#include <QFrame>
#include <QGuiApplication>
#include <QGroupBox>
#include <QCheckBox>
#include <QImage>
#include <QInputDialog>
#include <QListWidget>
#include <QMenu>
#include <QProgressBar>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QLinearGradient>
#include <QMenuBar>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QScrollBar>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStandardPaths>
#include <QStatusBar>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTemporaryDir>
#include <QTest>
#include <QTest>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QToolBar>
#include <QToolButton>
#include <QWidget>

#include <DockManager.h>
#include <DockWidget.h>

#include <algorithm>
#include <cmath>

namespace kerf {

class EditorTest final : public QObject {
    Q_OBJECT
    // Bring the Kerf Assistant tab to the front; hidden tabs are detached.
    static void showAssistant(MainWindow &window) {
        window.m_workspace->panel(QStringLiteral("assistant"))->toggleView(true);
        window.m_workspace->panel(QStringLiteral("assistant"))->setAsCurrentTab();
        QCoreApplication::processEvents();
    }

private slots:
    void nativePresetExportAndMediaSlots() {
        QTemporaryDir project;
        QVERIFY(project.isValid());
        MainWindow window{project.path(), QString()};
        const QString path = QDir(project.path()).filePath(QStringLiteral("style.kerfpreset"));
        const QString document = QStringLiteral("kerf 1\nproject name=\"Style\"\n");
        window.handleEvent(QJsonObject{{"type", "preset_export"}, {"path", path}, {"document", document}});
        QFile saved(path);
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), document.toUtf8());
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 7}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 30}, {"sources", QJsonArray{
                QJsonObject{{"id", "silent"}, {"path", "silent.mp4"}, {"width", 320}, {"duration", 4}, {"fps", 24}},
                QJsonObject{{"id", "replacement"}, {"path", "new.mp4"}, {"width", 320}, {"hasAudio", true}, {"duration", 4}, {"fps", 24}}}},
            {"tracks", QJsonArray{}}}}});
        QVERIFY(window.m_commands->action(QStringLiteral("file.exportPreset")));
        QVERIFY(window.m_commands->action(QStringLiteral("file.applyPreset")));
        QTimer::singleShot(0, &window, [&window, &document] {
            auto *dialog = window.findChild<QDialog *>(QStringLiteral("presetBindingsDialog"));
            QVERIFY(dialog);
            // Ensure assertion failures cannot strand the modal test dialog.
            QTimer::singleShot(2000, dialog, &QDialog::reject);
            auto *choice = dialog->findChild<QComboBox *>(QStringLiteral("presetMedia_slot1"));
            auto *start = dialog->findChild<QDoubleSpinBox *>(QStringLiteral("presetFrom_slot1"));
            auto *text = dialog->findChild<QLineEdit *>(QStringLiteral("presetText_title1"));
            auto *code = dialog->findChild<QPlainTextEdit *>(QStringLiteral("presetCode"));
            auto *buttons = dialog->findChild<QDialogButtonBox *>();
            QVERIFY(choice && start && text && code && buttons);
            QCOMPARE(choice->count(), 2); // Only media Kerf found able to fill the slot.
            QCOMPARE(choice->itemData(1).toString(), QStringLiteral("replacement"));
            QStringList uses;
            for (auto *label : dialog->findChildren<QLabel *>(QStringLiteral("muted"))) uses << label->text();
            QVERIFY(uses.contains(QStringLiteral("V1 00:00:02:00 “Opening” · child › A1 00:00:00:00")));
            QVERIFY(!buttons->button(QDialogButtonBox::Ok)->isEnabled());
            QCOMPARE(code->toPlainText(), document);
            QVERIFY(code->isReadOnly());
            choice->setCurrentIndex(1);
            QCOMPARE(start->maximum(), 1.958); // Frame 47 at 24 fps, shown to the millisecond.
            QCOMPARE(start->singleStep(), 1.0 / 24);
            start->setValue(1.25);
            text->setText(QStringLiteral("New title"));
            text->setModified(true);
            QVERIFY(buttons->button(QDialogButtonBox::Ok)->isEnabled());
            buttons->button(QDialogButtonBox::Ok)->click();
        });
        window.showPresetBindings(path, QJsonObject{{"name", "Style"}, {"document", document}, {"sequence", "s1"},
            {"slots", QJsonArray{QJsonObject{{"id", "slot1"}, {"seconds", 2}, {"video", true}, {"audio", true},
                {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"name", "Opening"}, {"sequence", "s1"}, {"track", "V1"}, {"at", "00:00:02:00"}},
                    QJsonObject{{"id", "a1"}, {"sequence", "child"}, {"track", "A1"}, {"at", "00:00:00:00"}}}},
                {"fits", QJsonArray{QJsonObject{{"media", "replacement"}, {"latestFrame", 47}}}}}}},
            {"textSlots", QJsonObject{{"title1", "Original"}}}});
        const QJsonObject request = window.m_lastCommand;
        QCOMPARE(request.value("type").toString(), QStringLiteral("preset_apply"));
        QCOMPARE(request.value("revision").toInt(), 7);
        QCOMPARE(request.value("bindings").toObject().value("slot1").toObject().value("media").toString(), QStringLiteral("replacement"));
        QCOMPARE(request.value("bindings").toObject().value("slot1").toObject().value("from").toString(), QStringLiteral("30f"));
        QCOMPARE(request.value("texts").toObject().value("title1").toString(), QStringLiteral("New title"));
    }

    void initTestCase() {
        QApplication::setStyle(QStringLiteral("Fusion"));
        // Tests draw in the dark appearance unless they pick one: offscreen
        // reports a light system, and painted colours are checked in dark.
        setEditorColorScheme(ColorScheme::Dark);
        QVERIFY(editorTheme().dark);
    }

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

    void viewerKeepsLandscapeAndShortsInsideOneCanvasContract() {
        ViewerWidget viewer;
        viewer.resize(800, 600);

        viewer.setProjectInfo(1920, 1080);
        const QRectF landscape = viewer.videoRect();
        QVERIFY(std::abs(landscape.width() / landscape.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(viewer.rect().contains(landscape.toAlignedRect()));
        QVERIFY(landscape.contains(viewer.captionRect()));

        viewer.setProjectInfo(1080, 1920);
        const QRectF shorts = viewer.videoRect();
        QVERIFY(std::abs(shorts.width() / shorts.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(viewer.rect().contains(shorts.toAlignedRect()));
        QVERIFY(shorts.contains(viewer.captionRect()));

        viewer.resize(800, 220);
        viewer.setProjectInfo(1920, 1080);
        QVERIFY(viewer.videoRect().top() < 20.0);
        viewer.show();
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QTest::mouseMove(&viewer, QPoint(400, 80));
        QVERIFY(!scrubs.isEmpty());
        QCOMPARE(scrubs.constLast()[0].toDouble(), 0.5);
        QVERIFY(nearerScrubFrame(1.0, true, 2.0, 3.0, 24.0));
        QVERIFY(!nearerScrubFrame(2.5, true, 0.2, 3.0, 24.0));
        QVERIFY(nearerScrubFrame(0.0, false, 9.0, 1.0, 24.0));
    }

    void viewerCanvasResolutionMatchesProjectThenSourceThenDefault() {
        QCOMPARE(resolvedCanvasSize(QJsonObject{{"width", 1920}, {"height", 1080}},
                                    QJsonArray{QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1920, 1080));
        QCOMPARE(resolvedCanvasSize(QJsonObject{},
                                    QJsonArray{QJsonObject{{"width", 1280}, {"height", 720}},
                                               QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1080, 1920));
        QCOMPARE(resolvedCanvasSize(QJsonObject{}, QJsonArray{}), QSize(1920, 1080));
    }

    void agentPayloadFollowsSettingsProtocol() {
        MainWindow window{QString(), QString()};
        showAssistant(window);
        QVERIFY(!window.findChild<QComboBox *>(QStringLiteral("agentProvider")));
        QVERIFY(!window.findChild<QFrame *>(QStringLiteral("agentContext")));
        QVERIFY(!window.findChild<QLabel *>(QStringLiteral("agentConfig")));
        QVERIFY(window.findChild<QComboBox *>(QStringLiteral("agentModel")));
        // Provider details live in Settings, not beside the chat.
        QWidget *chatTab = window.findChild<QWidget *>(QStringLiteral("assistantPanel"));
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
        QVERIFY(std::all_of(window.m_transportButtons.cbegin(), window.m_transportButtons.cend(),
                            [](const QToolButton *button) { return button->isEnabled(); }));
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(std::none_of(window.m_transportButtons.cbegin(), window.m_transportButtons.cend(),
                             [](const QToolButton *button) { return button->isEnabled(); }));
        QCOMPARE(window.m_workspace->panel(QStringLiteral("assistant"))->windowTitle(), QStringLiteral("Kerf Assistant"));

        auto *composer = window.findChild<QFrame *>(QStringLiteral("chatComposer"));
        QVERIFY(composer);
        QToolButton *activity = nullptr;
        for (auto *button : window.findChildren<QToolButton *>()) {
            if (button->text().endsWith(QStringLiteral("Activity"))) {
                activity = button;
                break;
            }
        }
        QVERIFY(activity);
        QVERIFY(activity->mapTo(&window, QPoint{}).y() < composer->mapTo(&window, QPoint{}).y());
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

    void conversationSeparatesActivityAndKeepsStreamInItsMessage() {
        MainWindow window{QString(), QString()};
        window.show();
        showAssistant(window);
        QVERIFY(window.m_conversation->toPlainText().contains(QStringLiteral("Make your next cut.")));
        const QString emptyState = window.m_conversation->toPlainText();
        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Inspecting timeline"));
        QCOMPARE(window.m_conversation->toPlainText(), emptyState);
        QVERIFY(!window.m_chatActivity->isHidden());

        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim <intro>\nKeep the dialogue."));
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "I’ll trim "}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "Reading <timeline>"}});
        window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "failed"}, {"message", "Could not read <media>"}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "the opening."}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "I’ll trim the opening."}});

        const QString transcript = window.m_conversation->toPlainText();
        QVERIFY(!transcript.contains(QStringLiteral("Make your next cut.")));
        QVERIFY(transcript.contains(QStringLiteral("Trim <intro>\nKeep the dialogue.")));
        QCOMPARE(transcript.count(QStringLiteral("I’ll trim the opening.")), 1);
        QVERIFY(transcript.contains(QStringLiteral("Could not read <media>")));
        QVERIFY(!transcript.contains(QStringLiteral("Reading <timeline>")));
        QCOMPARE(window.m_activityList->count(), 1);
        const auto frames = window.m_conversation->document()->rootFrame()->childFrames();
        QCOMPARE(frames.size(), 3);
        QVERIFY(frames[0]->frameFormat().leftMargin() > frames[1]->frameFormat().leftMargin());
        QVERIFY(frames[1]->frameFormat().borderBrush() != frames[2]->frameFormat().borderBrush());
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("A separate response."));
        QCOMPARE(window.m_conversation->document()->rootFrame()->childFrames().size(), 4);

        // New output must not pull the reader away from earlier messages.
        for (int i = 0; i < 12; ++i)
            window.appendConversation(QStringLiteral("You"), QStringLiteral("A longer edit request.\nKeep this line readable."));
        QCoreApplication::processEvents();
        auto *scroll = window.m_conversation->verticalScrollBar();
        QVERIFY(scroll->maximum() > 0);
        scroll->setValue(0);
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("New response."));
        QCOMPARE(scroll->value(), 0);
    }

    void composerStatesAndNarrowLayoutRemainUsable() {
        MainWindow window{QString(), QString()};
        window.resize(1080, 700);
        window.show();
        showAssistant(window);
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("offline"));
        window.setConnection(QStringLiteral("ready"));
        window.activateWindow();
        window.m_chatInput->setFocus();
        QTRY_COMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("focused"));
        window.m_chatInput->setPlainText(QStringLiteral("Keep my draft"));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("sending"));
        QVERIFY(!window.m_chatInput->isEnabled());
        QVERIFY(window.m_abortButton->isVisible());
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QVERIFY(window.m_chatInput->isEnabled());
        QVERIFY(window.m_sendButton->isEnabled());
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Keep my draft"));
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim the opening to three seconds."));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I’ll shorten the opening and keep the rest of your sequence in place.\nReview the changes before applying them."));
        window.appendConversationNote(QStringLiteral("Tool"), QString(500, QLatin1Char('x')));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "review"},
            {"script", "trim c1 out -1s\n"}, {"documentDiff", "-    clip c1 dur=00:00:04:00\n+    clip c1 dur=00:00:03:00\n"}});
        window.m_activityList->show();
        QCoreApplication::processEvents();
        QVERIFY(window.m_chatInput->height() <= 36);
        auto *chat = window.m_chatComposer->parentWidget();
        QVERIFY(chat->rect().contains(window.m_chatComposer->geometry()));
        QCOMPARE(window.m_chatComposer->geometry().bottom(), chat->rect().bottom());
        QVERIFY(window.m_conversation->height() >= 80);
        QCOMPARE(window.m_conversation->horizontalScrollBar()->maximum(), 0);
        QVERIFY(window.m_proposalBox->rect().contains(window.m_proposalDiff->geometry()));
        QVERIFY(window.m_proposalDiff->geometry().bottom() < window.m_acceptButton->geometry().top());
        QVERIFY(window.m_proposalBox->rect().contains(window.m_acceptButton->geometry()));
        auto *reviewScroll = window.m_proposalBox->parentWidget()->parentWidget()->parentWidget();
        QVERIFY(reviewScroll->mapTo(chat, QPoint(0, reviewScroll->height())).y() <= window.m_chatComposer->y());
        // Optional artifact for manual review of the real styled widget tree.
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            QVERIFY(window.grab().save(screenshot));
            window.resize(1500, 940);
            window.m_proposalBox->hide();
            window.m_activityList->hide();
            window.appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Response complete"));
            QCoreApplication::processEvents();
            QVERIFY(window.grab().save(screenshot + QStringLiteral(".chat.png")));
        }
    }

    void timelineFillsTheRemovedSourceRow() {
        MainWindow window{QString(), QString()};
        window.resize(1280, 800);
        window.show();
        QTest::qWait(30);
        QVERIFY(window.findChild<QWidget *>(QStringLiteral("sourceTray")) == nullptr);
        // The viewer panel (with its transport) meets the timeline panel directly.
        auto *viewerPanel = window.findChild<QWidget *>(QStringLiteral("viewerFrame"));
        auto *timelinePanel = window.findChild<QWidget *>(QStringLiteral("timelinePanel"));
        QVERIFY(viewerPanel && timelinePanel);
        const int viewerBottom = viewerPanel->mapTo(&window, QPoint(0, viewerPanel->height())).y();
        const int timelineTop = timelinePanel->mapTo(&window, QPoint(0, 0)).y();
        // Only the timeline's title strip sits between them.
        QVERIFY(timelineTop >= viewerBottom - 4);
        QVERIFY(timelineTop <= viewerBottom + 36);
        QVERIFY(window.m_timelineScroll->height() >= 280);
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(QJsonObject{{"duration", 8}, {"timeline", QJsonObject{
                {"name", "Cut"}, {"fps", 24}, {"tracks", QJsonArray{}}
            }}});
            QVERIFY(window.grab().save(screenshot + QStringLiteral(".empty.png")));
            const QString sample = qEnvironmentVariable("KERF_QT_UI_SAMPLE_VIDEO");
            if (!sample.isEmpty()) {
                const QJsonArray sources{QJsonObject{{"id", "sample"}, {"path", sample}}};
                window.applySnapshot(QJsonObject{{"duration", 4}, {"timeline", QJsonObject{
                    {"name", "Cut"}, {"fps", 24},
                    {"sources", sources},
                    {"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "sample-clip"}, {"sourceId", "sample"}, {"timelineStart", 0},
                                    {"duration", 4}, {"in", 0}, {"out", 4}}}}}}}
                }}});
                QTRY_VERIFY_WITH_TIMEOUT(window.m_artwork->frames.contains(QStringLiteral("sample")), 5000);
                window.m_viewer->setFrame(window.m_artwork->frames.value(QStringLiteral("sample")), 0);
                QVERIFY(window.grab().save(screenshot));
            }
        }
    }

    void timelineEditsAreTypedOperationsOnTheModel() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 1.0},
                            {"duration", 2.0}, {"startFrame", 24}, {"durationFrames", 48},
                            {"in", 5.0}, {"out", 7.0}, {"speed", 1.0}}}}}}}}}});
        auto op = [&window] {
            const QJsonObject command = window.m_lastCommand;
            if (command.value("type").toString() != QStringLiteral("edit")) return QJsonObject{};
            if (command.value("revision").toInt() != 3) return QJsonObject{};
            return command.value("ops").toArray().first().toObject();
        };
        window.moveClip(QStringLiteral("c1"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("move"));
        QCOMPARE(op().value("frames").toInt(), 12);
        window.trimClip(QStringLiteral("c1"), false, 66);
        QCOMPARE(op().value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op().value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(op().value("frames").toInt(), -6);
        window.trimClip(QStringLiteral("c1"), true, 30);
        QCOMPARE(op().value("edge").toString(), QStringLiteral("in"));
        QCOMPARE(op().value("frames").toInt(), 6);
        window.selectClip(QStringLiteral("c1"));
        window.m_playhead = 2.0;
        window.splitClip();
        QCOMPARE(op().value("op").toString(), QStringLiteral("split"));
        QCOMPARE(op().value("at").toInt(), 48);
        window.deleteSelected();
        QCOMPARE(op().value("clips").toArray().first().toString(), QStringLiteral("c1"));
        const QJsonObject crop = MainWindow::cropProps(QJsonObject{{"x", 0.25}, {"y", 0.0}, {"width", 0.5}, {"height", 1.0}});
        QCOMPARE(crop.value("crop_l").toDouble(), 0.25);
        QCOMPARE(crop.value("crop_r").toDouble(), 0.25);
        QVERIFY(MainWindow::cropProps(QJsonObject{}).value("crop_l").isNull());
        const QJsonObject box = MainWindow::cropBox(QJsonObject{{"props", crop}});
        QCOMPARE(box.value("x").toDouble(), 0.25);
        QCOMPARE(box.value("width").toDouble(), 0.5);
        QVERIFY(MainWindow::cropBox(QJsonObject{}).isEmpty());
    }

    void commandsBuildMenusShortcutsAndRequests() {
        MainWindow window{QString(), QString()};
        QStringList menus;
        for (const QAction *menu : window.menuBar()->actions()) menus.append(menu->text());
        QCOMPARE(menus, (QStringList{"File", "Edit", "Clip", "Sequence", "Markers", "Playback", "View", "Collaborate", "Window"}));
        QVERIFY(window.findChild<QAction *>(QStringLiteral("keymap.premiere")));
        // Projects open from File; Import Media keeps Ctrl+O.
        QVERIFY(window.findChild<QMenu *>(QStringLiteral("openRecentMenu")));
        QCOMPARE(window.m_commands->action(QStringLiteral("file.openProject"))->shortcut(),
                 QKeySequence::fromString(QStringLiteral("Ctrl+Shift+O"), QKeySequence::PortableText));
        QCOMPARE(window.m_commands->action(QStringLiteral("file.import"))->shortcut(),
                 QKeySequence::fromString(QStringLiteral("Ctrl+O"), QKeySequence::PortableText));
        QVERIFY(window.m_commands->action(QStringLiteral("collab.switchBranch"))->menu());

        QAction *split = window.m_commands->action(QStringLiteral("clip.split"));
        QAction *undo = window.m_commands->action(QStringLiteral("edit.undo"));
        QVERIFY(split && undo);
        // Unavailable commands say why.
        QVERIFY(!split->isEnabled());
        QVERIFY(split->toolTip().contains(QStringLiteral("Waiting for the project connection")));

        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"canUndo", true}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"timelineStart", 1.0}, {"duration", 2.0},
                            {"startFrame", 24}, {"durationFrames", 48}}}}}}}}}});
        QCOMPARE(window.m_commands->reason(QStringLiteral("clip.split")), QStringLiteral("Select a clip first"));
        QVERIFY(undo->isEnabled());
        undo->trigger();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("undo"));
        QCOMPARE(window.m_lastCommand.value("revision").toInt(), 3);

        window.selectClip(QStringLiteral("c1"));
        window.queueSeek(2.0);
        QVERIFY(split->isEnabled());
        window.m_lastCommand = {};
        window.show();
        window.m_timeline->setFocus();
        QTRY_VERIFY(window.m_timeline->hasFocus());
        QTest::keyClick(window.m_timeline, Qt::Key_S);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("split"));
        QCOMPARE(op.value("at").toInt(), 48);

        // Keymaps rebind the same actions; transport tooltips follow.
        const QKeySequence portable = QKeySequence::fromString(QStringLiteral("Ctrl+K"), QKeySequence::PortableText);
        window.m_commands->setKeymap(QStringLiteral("premiere"));
        QCOMPARE(split->shortcut(), portable);
        window.m_commands->setKeymap(QStringLiteral("kerf"));
        QCOMPARE(split->shortcut(), QKeySequence(Qt::Key_S));
        QVERIFY(window.m_transportPlay->toolTip().startsWith(QStringLiteral("Play (")));
    }

    void workspacesDockPanelsAndRememberLayouts() {
        QSettings().remove(QStringLiteral("workspace"));
        {
            MainWindow window{QString(), QString()};
            window.show();
            Workspace *workspace = window.m_workspace;
            ads::CDockWidget *timeline = workspace->panel(QStringLiteral("timeline"));
            ads::CDockWidget *project = workspace->panel(QStringLiteral("project"));
            QAction *toggle = window.m_commands->action(QStringLiteral("window.timeline"));
            QVERIFY(toggle->isChecked());
            QVERIFY(!timeline->isClosed());
            toggle->trigger();
            QVERIFY(timeline->isClosed());
            // Reopening a panel from its own controls keeps the menu in step.
            timeline->toggleView(true);
            QVERIFY(toggle->isChecked());

            QCOMPARE(workspace->names(), QStringList{QStringLiteral("Editing")});
            // Kerf is asked from the timeline; its conversation panel starts closed.
            QVERIFY(workspace->panel(QStringLiteral("assistant"))->isClosed());
            QVERIFY(workspace->saveAs(QStringLiteral("Review")));
            QVERIFY(!workspace->saveAs(QStringLiteral("review")));
            QCOMPARE(workspace->current(), QStringLiteral("Review"));
            project->toggleView(false);
            workspace->activate(QStringLiteral("Editing"));
            QVERIFY(!project->isClosed());
            // Each workspace keeps its changes until it is reset.
            workspace->activate(QStringLiteral("Review"));
            QVERIFY(project->isClosed());
            QVERIFY(!window.m_commands->action(QStringLiteral("window.project"))->isChecked());
            workspace->resetCurrent();
            QVERIFY(!project->isClosed());

            window.m_commands->action(QStringLiteral("app.preferences"))->trigger();
            QVERIFY(window.m_preferences->isVisible());
        }
        QSettings().remove(QStringLiteral("workspace"));
    }

    void effectsPanelShowsCardsAndAddsToTheSelectedClip() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        // The Editing layout keeps Effects behind the Project bin.
        QVERIFY(!window.m_workspace->panel(QStringLiteral("effects"))->isClosed());
        QVERIFY(window.m_commands->action(QStringLiteral("window.effects")));
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}}}}}}}}}});
        EffectsPanel *panel = window.m_effects;
        // The whole effects catalog is offered, by category and words.
        QVERIFY(panel->m_items.size() > 100);
        QVERIFY(panel->m_items.contains(QStringLiteral("light.edgeglow")));
        QVERIFY(!panel->m_items.contains(QStringLiteral("push")));
        panel->m_search->setText(QStringLiteral("neon"));
        QVERIFY(!panel->m_items.value(QStringLiteral("light.edgeglow"))->isHidden());
        QVERIFY(panel->m_items.value(QStringLiteral("channel.invert"))->isHidden());
        window.selectClip(QStringLiteral("c1"));
        QCOMPARE(panel->m_clip, QStringLiteral("c1"));
        QVERIFY(panel->m_picture);
        // The cards shown ask for previews of themselves on the clip.
        panel->requestPreviews();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("c1"));
        QVERIFY(window.m_lastCommand.value("effects").toArray().contains(QJsonValue("light.edgeglow")));
        QImage red(16, 9, QImage::Format_RGBA8888);
        red.fill(Qt::red);
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", "c1"}, {"fx", "light.edgeglow"},
                                       {"width", 16}, {"height", 9},
                                       {"rgba", QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(red.constBits()), 16 * 9 * 4).toBase64())}});
        const QImage card = panel->m_items.value(QStringLiteral("light.edgeglow"))->icon().pixmap(QSize(160, 90)).toImage();
        QCOMPARE(card.pixelColor(card.width() / 2, card.height() / 2), QColor(Qt::red));
        // Double-click adds the effect to the selected clip.
        emit panel->m_cards->itemActivated(panel->m_items.value(QStringLiteral("light.edgeglow")));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(op.value("fx").toString(), QStringLiteral("light.edgeglow"));
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
        QVERIFY(!window.m_aiBar->isHidden());
        QCOMPARE(window.m_aiBarText->text(), QStringLiteral("Next with Kerf"));
        QCOMPARE(window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep")).size(), 2);
        // Pressing one asks for it, about what the last request was about.
        chips.at(1)->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value("text").toString(), QStringLiteral("Use a linear wipe into the next shot."));
        QCOMPARE(window.m_lastCommand.value("focus").toObject(), focus);
    }

    void inspectorShowsAndEditsTheEffectStack() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}, {"effects", QJsonArray{
                                    QJsonObject{{"fx", "light.edgeglow"}, {"enabled", true}, {"params", QJsonObject{{"radius", 40}}}},
                                    QJsonObject{{"fx", "channel.invert"}, {"enabled", false}, {"params", QJsonObject{}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        EffectControlsPanel *inspector = window.m_effectControls;
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 2);
        auto *glow = boxes.at(0)->findChild<QCheckBox *>(QStringLiteral("effectEnabled"));
        auto *radius = boxes.at(0)->findChild<QDoubleSpinBox *>(QStringLiteral("effectParam_radius"));
        QVERIFY(glow && radius);
        QCOMPARE(glow->text(), QStringLiteral("Edge Glow"));
        QVERIFY(glow->isChecked());
        QCOMPARE(radius->value(), 40.0);
        QVERIFY(!boxes.at(1)->findChild<QCheckBox *>(QStringLiteral("effectEnabled"))->isChecked());
        auto op = [&window] { return window.m_lastCommand.value("ops").toArray().first().toObject(); };
        glow->click();
        QCOMPARE(op().value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(op().value("index").toInt(), 0);
        QCOMPARE(op().value("enabled").toBool(), false);
        radius->setValue(10.0); // the default
        emit radius->editingFinished();
        QCOMPARE(op().value("params").toObject().value("radius"), QJsonValue());
        boxes.at(1)->findChild<QToolButton *>(QStringLiteral("effectRemove"))->click();
        QCOMPARE(op().value("op").toString(), QStringLiteral("fx-remove"));
        QCOMPARE(op().value("index").toInt(), 1);
    }

    // Mouse events sent straight to `widget`, which need not be on screen.
    static void mouse(QWidget *widget, QEvent::Type type, const QPointF &at) {
        const Qt::MouseButton button = type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton;
        const Qt::MouseButtons buttons = type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::MouseButtons(Qt::LeftButton);
        QMouseEvent event(type, at, widget->mapToGlobal(at), button, buttons, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &event);
    }
    static void key(QWidget *widget, int code) {
        QKeyEvent event(QEvent::KeyPress, code, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &event);
    }
    static bool near(const QJsonValue &values, std::initializer_list<double> expected) {
        const QJsonArray array = values.toArray();
        if (array.size() != static_cast<int>(expected.size())) return false;
        int index = 0;
        for (double value : expected)
            if (std::abs(array.at(index++).toDouble() - value) > 1e-3) return false;
        return true;
    }

    void maskPicturePlacementFollowsTheCompositor() {
        // A 4:3 picture fitted to a 16:9 canvas is pillarboxed.
        const QSizeF canvas(1920, 1080);
        const QTransform fitted = picturePlacement(QJsonObject{}, QSizeF(640, 480), true, canvas, 0.0);
        QCOMPARE(fitted.map(QPointF(0, 0)), QPointF(240, 0));
        QCOMPARE(fitted.map(QPointF(1, 1)), QPointF(1680, 1080));
        // Scale about the center, then move by frame widths.
        const QTransform moved = picturePlacement(QJsonObject{{"props", QJsonObject{{"scale", 0.5}, {"x", 0.25}}}},
                                                  QSizeF(1920, 1080), true, canvas, 0.0);
        QCOMPARE(moved.map(QPointF(0.5, 0.5)), QPointF(1440, 540));
        QCOMPARE(moved.map(QPointF(0, 0)), QPointF(960, 270));
        // Rotation is clockwise about the anchor.
        const QPointF right = picturePlacement(QJsonObject{{"props", QJsonObject{{"rotation", 90.0}}}},
                                               QSizeF(1920, 1080), true, canvas, 0.0).map(QPointF(1, 0.5));
        QVERIFY(std::abs(right.x() - 960) < 1e-6 && std::abs(right.y() - 1500) < 1e-6);
        // A crop keeps the picture's fractions where they were in the source.
        const QTransform cropped = picturePlacement(QJsonObject{{"props", QJsonObject{{"crop_l", 0.5}}}},
                                                    QSizeF(1920, 1080), false, canvas, 0.0);
        QCOMPARE(cropped.map(QPointF(0.5, 0.5)), QPointF(960 - 480, 540));
        // Animated properties are read at the clip time.
        const QJsonObject keyed{{"x", QJsonArray{QJsonObject{{"at", 0.0}, {"value", 0.0}}, QJsonObject{{"at", 2.0}, {"value", 1.0}}}}};
        QCOMPARE(propertyAt(keyed, "x", 1.0, 0.0), 0.5);
        QCOMPARE(propertyAt(keyed, "x", 3.0, 0.0), 1.0);
        QCOMPARE(propertyAt(keyed, "y", 1.0, 0.25), 0.25);
        QVERIFY(maskPath(QJsonObject{{"points", QJsonArray{QJsonArray{0, 0}, QJsonArray{1, 0}, QJsonArray{1, 1}}}, {"closed", false}})
                    .elementCount() == 3);
    }

    void viewerDrawsAndEditsMasksAsTypedOperations() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonArray maskProperties{
            QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0.0}, {"min", 0.0}, {"max", 10000.0}, {"unit", "pixels"}},
            QJsonObject{{"name", "opacity"}, {"type", "number"}, {"default", 1.0}, {"min", 0.0}, {"max", 1.0}}};
        const auto snapshot = [&window, &maskProperties](int revision, const QJsonArray &masks) {
            QJsonObject clip{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                             {"effects", QJsonArray{QJsonObject{{"fx", "generate.audiowaveform"}, {"enabled", true}, {"params", QJsonObject{}}}}}};
            if (!masks.isEmpty()) clip.insert("masks", masks);
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 2}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1920}, {"height", 1080}, {"maskProperties", maskProperties},
                {"maskModes", QJsonArray{"add", "subtract", "none"}},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 2}}}},
                {"tracks", QJsonArray{
                    QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                    QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "a1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        };
        const QJsonObject k1{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "add"},
                             {"inverted", false}, {"params", QJsonObject{}}};
        auto op = [&window] {
            const QJsonObject command = window.m_lastCommand;
            window.m_lastCommand = {};
            return command.value("type").toString() == QStringLiteral("edit")
                ? command.value("ops").toArray().first().toObject() : QJsonObject{};
        };
        snapshot(3, {});
        ViewerWidget *viewer = window.m_viewer;
        viewer->resize(960, 560);
        auto *rectangle = window.findChild<QAction *>(QStringLiteral("maskRectangle"));
        auto *ellipse = window.findChild<QAction *>(QStringLiteral("maskEllipse"));
        auto *pen = window.findChild<QAction *>(QStringLiteral("maskPen"));
        QVERIFY(rectangle && ellipse && pen);
        QVERIFY(!rectangle->isEnabled());
        // A sound clip has no picture to mask.
        window.selectClip(QStringLiteral("a1"));
        QVERIFY(!rectangle->isEnabled());
        QVERIFY(!window.m_effectControls->m_masksSection->isVisibleTo(window.m_effectControls));
        window.selectClip(QStringLiteral("c1"));
        QVERIFY(rectangle->isEnabled());
        QVERIFY(window.m_effectControls->m_masksSection->isVisibleTo(window.m_effectControls));
        QCOMPARE(window.m_effectControls->findChildren<QToolButton *>(QStringLiteral("maskTool")).size(), 3);
        // With no masks and no tool, the viewer still scrubs.
        QSignalSpy scrubs(viewer, &ViewerWidget::scrubbed);
        mouse(viewer, QEvent::MouseMove, QPointF(300, 200));
        QCOMPARE(scrubs.size(), 1);

        // A drag with the rectangle tool adds k1 where it was drawn, then
        // puts the tool down with the new mask selected.
        rectangle->trigger();
        QCOMPARE(viewer->maskTool(), MaskTool::Rectangle);
        QTransform toWidget = viewer->pictureToWidget();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.25, 0.25)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.5, 0.6)));
        QCOMPARE(scrubs.size(), 1);
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.75, 0.75)));
        QJsonObject added = op();
        QCOMPARE(added.value("op").toString(), QStringLiteral("mask-add"));
        QCOMPARE(added.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(added.value("name").toString(), QStringLiteral("k1"));
        QVERIFY2(near(added.value("rect"), {0.25, 0.25, 0.75, 0.75}), qPrintable(QJsonDocument(added).toJson()));
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(!rectangle->isChecked());
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        // A snapshot read before the edit landed does not drop the selection.
        snapshot(3, {});
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));

        // The snapshot carries it. A corner drags on its own, and a drag
        // inside moves the whole mask.
        snapshot(4, QJsonArray{k1});
        QCOMPARE(viewer->m_masks.size(), 1);
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        toWidget = viewer->pictureToWidget();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.75, 0.75)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.9, 0.8)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.9, 0.8)));
        QJsonObject set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("name").toString(), QStringLiteral("k1"));
        QVERIFY2(near(set.value("rect"), {0.25, 0.25, 0.9, 0.8}), qPrintable(QJsonDocument(set).toJson()));
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.5, 0.5)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.6, 0.5)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.6, 0.5)));
        set = op();
        QVERIFY2(near(set.value("rect"), {0.35, 0.25, 1.0, 0.8}), qPrintable(QJsonDocument(set).toJson()));
        // A click without a drag changes nothing; the picture holds still
        // while masks are shown.
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.5, 0.5)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.5, 0.5)));
        QVERIFY(op().isEmpty());
        mouse(viewer, QEvent::MouseMove, QPointF(20, 20));
        QCOMPARE(scrubs.size(), 1);

        // Effect Controls edits its mode, inversion, and properties, and an
        // effect's mask parameter picks it.
        snapshot(5, QJsonArray{k1});
        EffectControlsPanel *inspector = window.m_effectControls;
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
        QCOMPARE(boxes.size(), 1);
        QVERIFY(boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->isChecked());
        auto *mode = boxes.at(0)->findChild<QComboBox *>(QStringLiteral("maskMode"));
        auto *inverted = boxes.at(0)->findChild<QCheckBox *>(QStringLiteral("maskInverted"));
        auto *feather = boxes.at(0)->findChild<QDoubleSpinBox *>(QStringLiteral("maskParam_feather"));
        QVERIFY(mode && inverted && feather);
        QCOMPARE(mode->count(), 3);
        QCOMPARE(mode->currentText(), QStringLiteral("add"));
        mode->setCurrentIndex(2);
        emit mode->activated(2);
        set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("mode").toString(), QStringLiteral("none"));
        inverted->click();
        QCOMPARE(op().value("inverted").toBool(), true);
        feather->setValue(12.0);
        emit feather->editingFinished();
        QCOMPARE(op().value("params").toObject().value("feather").toDouble(), 12.0);
        feather->setValue(0.0);
        emit feather->editingFinished();
        QVERIFY(op().value("params").toObject().value("feather").isNull());
        auto *source = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_path"));
        QVERIFY(source);
        QCOMPARE(source->count(), 2);
        QCOMPARE(source->itemText(1), QStringLiteral("k1"));
        // A mask with mode none is a path only, still drawn and edited.
        snapshot(6, QJsonArray{QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "none"},
                                           {"inverted", false}, {"params", QJsonObject{{"feather", 12.0}}}}});
        QCOMPARE(mode->currentText(), QStringLiteral("none"));
        QCOMPARE(feather->value(), 12.0);
        QCOMPARE(viewer->maskAt(toWidget.map(QPointF(0.5, 0.5))), QStringLiteral("k1"));
        // Choosing it in the panel selects it on the viewer; Delete there
        // removes it, as ✕ in the panel does.
        viewer->selectMask(QString());
        boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->click();
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        key(viewer, Qt::Key_Delete);
        QJsonObject removed = op();
        QCOMPARE(removed.value("op").toString(), QStringLiteral("mask-remove"));
        QCOMPARE(removed.value("name").toString(), QStringLiteral("k1"));
        snapshot(7, QJsonArray{k1});
        inspector->findChild<QToolButton *>(QStringLiteral("maskRemove"))->click();
        QCOMPARE(op().value("op").toString(), QStringLiteral("mask-remove"));

        // The pen closes an outline on its first point, and Return leaves
        // it open; names take the next free number.
        snapshot(8, {});
        QVERIFY(inspector->findChildren<QWidget *>(QStringLiteral("maskBox")).isEmpty());
        pen->trigger();
        for (const QPointF &corner : {QPointF(0.1, 0.1), QPointF(0.9, 0.1), QPointF(0.5, 0.9), QPointF(0.1, 0.1)}) {
            mouse(viewer, QEvent::MouseButtonPress, toWidget.map(corner));
            mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(corner));
        }
        added = op();
        QCOMPARE(added.value("op").toString(), QStringLiteral("mask-add"));
        QCOMPARE(added.value("name").toString(), QStringLiteral("k1"));
        QCOMPARE(added.value("closed").toBool(), true);
        const QJsonArray points = added.value("points").toArray();
        QCOMPARE(points.size(), 3);
        QVERIFY(near(points.at(2), {0.5, 0.9}));
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        ellipse->trigger();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.2, 0.2)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.6, 0.8)));
        added = op();
        QCOMPARE(added.value("name").toString(), QStringLiteral("k2"));
        QVERIFY2(near(added.value("ellipse"), {0.4, 0.5, 0.2, 0.3}), qPrintable(QJsonDocument(added).toJson()));
        pen->trigger();
        for (const QPointF &point : {QPointF(0.1, 0.5), QPointF(0.9, 0.5)}) {
            mouse(viewer, QEvent::MouseButtonPress, toWidget.map(point));
            mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(point));
        }
        key(viewer, Qt::Key_Return);
        added = op();
        QCOMPARE(added.value("name").toString(), QStringLiteral("k3"));
        QCOMPARE(added.value("closed").toBool(), false);
        // Escape drops a path being drawn, then puts the tool down.
        pen->trigger();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.3, 0.3)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.3, 0.3)));
        key(viewer, Qt::Key_Escape);
        QVERIFY(viewer->m_penPoints.isEmpty());
        QCOMPARE(viewer->maskTool(), MaskTool::Pen);
        key(viewer, Qt::Key_Escape);
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(!pen->isChecked());
        QVERIFY(op().isEmpty());
        // Deselecting the clip hides its masks and stops a tool.
        rectangle->trigger();
        window.selectClip(QString());
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(viewer->m_maskClip.isEmpty());
        QVERIFY(!rectangle->isEnabled());
    }

    void inspectorEditsTheCatalogPropertiesOfTheTrackType() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject properties{
            {"video", QJsonArray{
                QJsonObject{{"name", "opacity"}, {"type", "number"}, {"default", 1.0}, {"min", 0.0}, {"max", 1.0}},
                QJsonObject{{"name", "x"}, {"type", "number"}, {"default", 0.0}, {"min", -10.0}, {"max", 10.0}},
                QJsonObject{{"name", "blend"}, {"type", "enum"}, {"default", "normal"}, {"options", QJsonArray{"normal", "add"}}}}},
            {"audio", QJsonArray{
                QJsonObject{{"name", "gain"}, {"type", "number"}, {"default", 0.0}, {"min", -96.0}, {"max", 24.0}, {"unit", "dB"}}}}};
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"properties", properties},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"opacity", 0.5}, {"x", QJsonArray{
                                    QJsonObject{{"at", 0.0}, {"value", 0.25}, {"interp", "linear"}},
                                    QJsonObject{{"at", 1.0}, {"value", 1.0}, {"interp", "linear"}}}}}}}}}},
                QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "g"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"text", "Day One"}}}}}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}},
            {"generators", QJsonObject{{"title", QJsonArray{
                QJsonObject{{"name", "text"}, {"type", "text"}, {"default", ""}},
                QJsonObject{{"name", "color"}, {"type", "color"}, {"default", "#ffffff"}}}}}}}}});
        auto set = [&window] {
            const QJsonObject command = window.m_lastCommand;
            window.m_lastCommand = {};
            const QJsonObject op = command.value("ops").toArray().first().toObject();
            return op.value("op").toString() == QStringLiteral("set") ? op : QJsonObject{};
        };
        QVERIFY(window.m_effectControls->m_propertyEditors.isEmpty());
        window.selectClip(QStringLiteral("v"));
        QCOMPARE(window.m_effectControls->m_propertyEditors.size(), 3);
        auto *opacity = qobject_cast<QDoubleSpinBox *>(window.m_effectControls->m_propertyEditors.value("opacity"));
        auto *x = qobject_cast<QDoubleSpinBox *>(window.m_effectControls->m_propertyEditors.value("x"));
        auto *blend = qobject_cast<QComboBox *>(window.m_effectControls->m_propertyEditors.value("blend"));
        QVERIFY(opacity && x && blend);
        QCOMPARE(opacity->value(), 0.5);
        QCOMPARE(opacity->maximum(), 1.0);
        QVERIFY(!x->isEnabled());
        QCOMPARE(x->value(), 0.25);
        QCOMPARE(blend->currentText(), QStringLiteral("normal"));
        window.m_effectControls->propertyEdited("opacity", 0.5);
        QVERIFY(set().isEmpty());
        window.m_effectControls->propertyEdited("opacity", 0.75);
        QCOMPARE(set().value("props").toObject().value("opacity").toDouble(), 0.75);
        window.m_effectControls->propertyEdited("opacity", 1.0);
        QVERIFY(set().value("props").toObject().value("opacity").isNull());
        window.m_effectControls->propertyEdited("blend", "add");
        QCOMPARE(set().value("props").toObject().value("blend").toString(), QStringLiteral("add"));
        window.selectClip(QStringLiteral("g"));
        QCOMPARE(window.m_effectControls->m_propertyEditors.size(), 5);
        auto *text = qobject_cast<QLineEdit *>(window.m_effectControls->m_propertyEditors.value("text"));
        auto *color = qobject_cast<QPushButton *>(window.m_effectControls->m_propertyEditors.value("color"));
        QVERIFY(text && color && window.m_effectControls->m_propertyEditors.contains("opacity"));
        QCOMPARE(text->text(), QStringLiteral("Day One"));
        QCOMPARE(color->text(), QStringLiteral("#ffffff"));
        QVERIFY(!color->icon().isNull());
        QCOMPARE(modelColor(QStringLiteral("#ff000080")).alpha(), 0x80);
        QCOMPARE(modelColorName(QColor(255, 0, 0, 0x80)), QStringLiteral("#ff000080"));
        QCOMPARE(modelColorName(Qt::white), QStringLiteral("#ffffff"));
        window.m_effectControls->propertyEdited("text", "Day Two");
        QCOMPARE(set().value("props").toObject().value("text").toString(), QStringLiteral("Day Two"));
        window.m_effectControls->propertyEdited("color", "#ffffff");
        QVERIFY(set().isEmpty());
        window.m_effectControls->propertyEdited("color", "#ff0000");
        QCOMPARE(set().value("props").toObject().value("color").toString(), QStringLiteral("#ff0000"));
        window.selectClip(QStringLiteral("a"));
        QCOMPARE(window.m_effectControls->m_propertyEditors.keys(), QList<QString>{"gain"});
        QVERIFY(window.m_effectControls->m_propertyEditors.value("gain")->toolTip().contains("dB"));
        window.m_effectControls->propertyEdited("gain", -6.0);
        QCOMPARE(set().value("props").toObject().value("gain").toDouble(), -6.0);
        // A file parameter is a line of text with a button that chooses the file.
        QWidget *file = window.m_effectControls->valueEditor(
            QJsonObject{{"name", "lut"}, {"type", "file"}, {"extensions", QJsonArray{"cube"}}, {"default", ""}},
            QStringLiteral("LUT"), [](const QJsonValue &) {});
        auto *fileLine = qobject_cast<QLineEdit *>(file);
        QVERIFY(fileLine);
        QCOMPARE(fileLine->actions().size(), 1);
        window.m_effectControls->setPropertyValue(file, QStringLiteral("looks/warm.cube"));
        QCOMPARE(fileLine->text(), QStringLiteral("looks/warm.cube"));
        delete file;
        // A track parameter picks a picture track other than the clip's, or
        // none; the sound clip `a` may read both.
        QCOMPARE(window.m_effectControls->m_pictureTracks, (QStringList{"V1", "V2"}));
        QWidget *track = window.m_effectControls->valueEditor(
            QJsonObject{{"name", "map"}, {"type", "track"}, {"default", ""}}, QStringLiteral("map"),
            [](const QJsonValue &) {});
        auto *trackCombo = qobject_cast<QComboBox *>(track);
        QVERIFY(trackCombo);
        QCOMPARE(trackCombo->count(), 3);
        window.m_effectControls->setPropertyValue(track, QStringLiteral("V2"));
        QCOMPARE(trackCombo->currentText(), QStringLiteral("V2"));
        delete track;
        // A sound parameter picks a sound track, or none.
        QCOMPARE(window.m_effectControls->m_soundTracks, QStringList{"A1"});
        QWidget *sound = window.m_effectControls->valueEditor(
            QJsonObject{{"name", "audioLayer"}, {"type", "sound"}, {"default", ""}}, QStringLiteral("audioLayer"),
            [](const QJsonValue &) {});
        auto *soundCombo = qobject_cast<QComboBox *>(sound);
        QVERIFY(soundCombo);
        QCOMPARE(soundCombo->count(), 2);
        delete sound;
    }

    void timelineKeepsSeekSelectMoveAndTrimCoordinates() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QSignalSpy selections(&timeline, &TimelineWidget::clipSelected);
        QSignalSpy moves(&timeline, &TimelineWidget::moveRequested);
        QSignalSpy trims(&timeline, &TimelineWidget::trimRequested);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 36, kTrackHeight / 2));
        QCOMPARE(seeks.constLast()[0].toDouble(), 0.5);
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 144, y));
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 216, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineLabelWidth + 216, y));
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("clip"));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        for (const bool leading : {true, false}) {
            const int edge = kTimelineLabelWidth + (leading ? 75 : 357);
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge, y));
            QTest::mouseMove(&timeline, QPoint(edge + 72, y));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge + 72, y));
            QCOMPARE(trims.constLast()[1].toBool(), leading);
            // The edge moves by the drag distance, landing where the preview showed it.
            QCOMPARE(trims.constLast()[2].toDouble(), double((leading ? 24 : 120) + 24));
        }
        QCOMPARE(trims.count(), 2);
        timeline.setSelectedClip(QStringLiteral("clip"));
        timeline.setPlayhead(2.5);
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
    }

    void sourceMarksInsertAndOverwriteOnTargetTracks() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "missing.mov"}, {"duration", 10.0},
                                               {"rate", "24"}, {"width", 1920}, {"height", 1080}, {"hasAudio", true}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{}}},
                                  QJsonObject{{"id", "A1"}, {"type", "audio"}, {"locked", true}, {"clips", QJsonArray{}}},
                                  QJsonObject{{"id", "A2"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}}}});
        QAction *insert = window.m_commands->action(QStringLiteral("source.insert"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("source.insert")), QStringLiteral("Open media in the source monitor"));
        window.openSource(QStringLiteral("m1"));
        window.m_sourceMonitor->seek(1.0);
        window.m_commands->action(QStringLiteral("source.markIn"))->trigger();
        window.m_sourceMonitor->seek(2.0);
        window.m_commands->action(QStringLiteral("source.markOut"))->trigger();
        QCOMPARE(window.m_sourceMonitor->inPoint(), 1.0);
        QVERIFY(insert->isEnabled());
        window.queueSeek(0.5);
        insert->trigger();
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("insert"));
        QCOMPARE(op.value("media").toString(), QStringLiteral("m1"));
        QCOMPARE(op.value("sourceIn").toInt(), 24);
        // The out mark includes the frame on screen.
        QCOMPARE(op.value("frames").toInt(), 25);
        QCOMPARE(op.value("at").toInt(), 12);
        // Locked tracks are not targets.
        QCOMPARE(op.value("tracks").toArray(), (QJsonArray{"V1", "A2"}));
        window.m_commands->action(QStringLiteral("source.overwrite"))->trigger();
        QCOMPARE(window.m_lastCommand.value("ops").toArray().first().toObject().value("op").toString(), QStringLiteral("overwrite"));
    }

    void sourceMonitorShowsTheFrameAtItsPosition() {
        QTemporaryDir directory;
        const QString video = directory.filePath(QStringLiteral("bars.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-f"), QStringLiteral("lavfi"),
            QStringLiteral("-i"), QStringLiteral("testsrc2=size=320x180:rate=24"), QStringLiteral("-t"), QStringLiteral("2"),
            QStringLiteral("-pix_fmt"), QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        SourceMonitorPanel monitor;
        monitor.open(QJsonObject{{"id", "bars"}, {"path", "bars.mp4"}, {"duration", 2.0}, {"rate", "24"}, {"width", 320}}, video);
        QTRY_VERIFY_WITH_TIMEOUT(!monitor.frame().isNull(), 10000);
        monitor.step(12);
        QCOMPARE(monitor.position(), 0.5);
        // Positions stay before the last frame.
        monitor.seek(9.0);
        QCOMPARE(monitor.position(), 2.0 - 1.0 / 24);
        monitor.markOut();
        QCOMPARE(monitor.outPoint(), 2.0);
        monitor.close();
        QVERIFY(!monitor.isOpen());
    }

    void toolsChooseTheEditAClipDragMakes() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 1.0}, {"startFrame", 0}, {"durationFrames", 24}},
                QJsonObject{{"id", "b"}, {"timelineStart", 1.0}, {"duration", 1.0}, {"startFrame", 24}, {"durationFrames", 24}}}}}}}}}});
        auto op = [&window] { return window.m_lastCommand.value("ops").toArray().first().toObject(); };
        window.m_commands->action(QStringLiteral("tool.slip"))->trigger();
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Slip);
        window.moveClip(QStringLiteral("b"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("slip"));
        QCOMPARE(op().value("frames").toInt(), -12);
        window.m_commands->action(QStringLiteral("tool.slide"))->trigger();
        window.moveClip(QStringLiteral("b"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("slide"));
        QCOMPARE(op().value("frames").toInt(), 12);
        window.m_commands->action(QStringLiteral("tool.ripple"))->trigger();
        window.trimClip(QStringLiteral("b"), false, 42);
        QCOMPARE(op().value("op").toString(), QStringLiteral("trim"));
        QVERIFY(op().value("ripple").toBool());
        // Rolling b's in point moves the cut after a.
        window.m_commands->action(QStringLiteral("tool.roll"))->trigger();
        window.trimClip(QStringLiteral("b"), true, 30);
        QCOMPARE(op().value("op").toString(), QStringLiteral("roll"));
        QCOMPARE(op().value("clip").toString(), QStringLiteral("a"));
        QCOMPARE(op().value("frames").toInt(), 6);
        // Only one tool is chosen at a time.
        QVERIFY(!window.m_commands->action(QStringLiteral("tool.slip"))->isChecked());
    }

    void timelineRazorSnapAndTrackToggles() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 2.0}},
                QJsonObject{{"id", "b"}, {"timelineStart", 4.0}, {"duration", 1.0}}}}}}}}, 8, 24);
        timeline.fitToWidth(720);
        timeline.show();
        QSignalSpy razors(&timeline, &TimelineWidget::razorRequested);
        QSignalSpy moves(&timeline, &TimelineWidget::moveRequested);
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        const int y = kRulerHeight + kTrackHeight / 2;
        const auto x = [&timeline](double seconds) {
            return kTimelineLabelWidth + int(std::lround(seconds * (timeline.width() - kTimelineLabelWidth) / 8.0));
        };
        timeline.setTool(TimelineTool::Razor);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(1.0), y));
        QCOMPARE(razors.count(), 1);
        QCOMPARE(razors[0][0].toString(), QStringLiteral("a"));
        QVERIFY(std::abs(razors[0][1].toDouble() - 24.0) <= 1.0);
        // Dragging b to just past a's end snaps it onto the cut.
        timeline.setTool(TimelineTool::Selection);
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(4.5), y));
        QTest::mouseMove(&timeline, QPoint(x(4.5) - (x(4.0) - x(2.0)) + 3, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(4.5) - (x(4.0) - x(2.0)) + 3, y));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        // Header toggles: picture output, then lock.
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerToggleRect(0, 0).center());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerToggleRect(0, 1).center());
        QCOMPARE(toggles.count(), 2);
        QCOMPARE(toggles[0][1].toString(), QStringLiteral("enabled"));
        QCOMPARE(toggles[0][2].toBool(), false);
        QCOMPARE(toggles[1][1].toString(), QStringLiteral("locked"));
        QCOMPARE(toggles[1][2].toBool(), true);
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
        // While Kerf works, the timeline says so and offers Stop.
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QVERIFY(window.m_aiBar->isVisible());
        QVERIFY(window.m_aiBarText->text().contains(QStringLiteral("Opening")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("ai.stop")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("ai.ask")), QStringLiteral("Kerf is working on a request"));
    }

    void proposalsShowOnTheTimelineAndAreReviewedInPlace() {
        MainWindow window{QString(), QString()};
        window.show();
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject current{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}};
        const QJsonObject proposed{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c1"}, {"timelineStart", 0.0}, {"duration", 1.0}}}}}}}};
        QJsonObject proposal{{"status", "pending"}, {"proposalId", "p1"}, {"script", "trim c1 out -24f\n"},
                             {"documentDiff", "-    clip c1 dur=2s\n+    clip c1 dur=1s\n"}};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 2}, {"timeline", current},
                                         {"pendingProposal", proposal}, {"proposalTimeline", proposed}});
        QVERIFY(window.m_timeline->hasProposal());
        QVERIFY(window.m_aiBar->isVisible());
        if (const QString shot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT"); !shot.isEmpty()) {
            window.resize(1500, 940);
            QCoreApplication::processEvents();
            QVERIFY(window.grab().save(shot + QStringLiteral(".proposal.png")));
        }
        QVERIFY(window.m_aiBarText->text().startsWith(QStringLiteral("Proposal")));
        QAction *accept = window.m_commands->action(QStringLiteral("proposal.accept"));
        QVERIFY(accept->isEnabled());
        accept->trigger();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("accept"));
        QCOMPARE(window.m_lastCommand.value("proposalId").toString(), QStringLiteral("p1"));
        QCOMPARE(window.m_lastCommand.value("revision").toInt(), 4);
        // Escape on the timeline rejects.
        window.m_timeline->setFocus();
        QTRY_VERIFY(window.m_timeline->hasFocus());
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("reject"));
        proposal.insert(QStringLiteral("freshness"), QStringLiteral("stale"));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 2}, {"timeline", current},
                                         {"pendingProposal", proposal}, {"proposalTimeline", proposed}});
        QVERIFY(!accept->isEnabled());
        window.applySnapshot(QJsonObject{{"revision", 6}, {"duration", 2}, {"timeline", current}});
        QVERIFY(!window.m_timeline->hasProposal());
        QVERIFY(!window.m_aiBar->isVisible());
    }

    void timelineDoubleClicksOpenOrAsk() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}}}}}, 8, 24);
        timeline.show();
        QSignalSpy opened(&timeline, &TimelineWidget::clipActivated);
        QSignalSpy asked(&timeline, &TimelineWidget::askRequested);
        const QPoint point = timeline.clipBounds(QStringLiteral("a")).center();
        QVERIFY(!timeline.clipBounds(QStringLiteral("a")).isEmpty());
        QTest::mouseDClick(&timeline, Qt::LeftButton, Qt::NoModifier, point);
        QTest::mouseDClick(&timeline, Qt::LeftButton, Qt::AltModifier, point);
        QCOMPARE(opened.count(), 1);
        QCOMPARE(asked.count(), 1);
        QCOMPARE(asked[0][0].toString(), QStringLiteral("a"));
        // End with the button up and the interval past, so later tests' clicks stand alone.
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, point);
        QTest::qWait(QApplication::doubleClickInterval() + 50);
    }

    void timelineDrawsSequenceMarkersOnTheRuler() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        const QJsonArray markers{
            QJsonObject{{"id", "beat1"}, {"time", 2.0}, {"duration", 0.0}, {"color", "red"}},
            QJsonObject{{"id", "chorus"}, {"time", 4.0}, {"duration", 1.0}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}, {"markers", markers}}, 8, 24);
        // A marker on the playhead is drawn over its tab.
        timeline.setPlayhead(2.0);
        timeline.show();
        const QRect beat = timeline.markerBounds(QStringLiteral("beat1"));
        const QRect chorus = timeline.markerBounds(QStringLiteral("chorus"));
        QVERIFY(!beat.isEmpty());
        QVERIFY(timeline.markerBounds(QStringLiteral("nowhere")).isEmpty());
        QVERIFY(beat.bottom() < kRulerHeight);
        // A marker with a duration spans it; one without is a tab.
        QVERIFY(chorus.width() > beat.width() + 10);
        QVERIFY(beat.right() < chorus.left());
        const QImage image = timeline.grab().toImage();
        QCOMPARE(image.pixelColor(beat.center().x(), beat.top() + 2), QColor("#e5534b"));
        QCOMPARE(image.pixelColor(chorus.left() + 4, chorus.top() + 2), QColor("#5fb85f"));
    }

    void timelineRevealsAddLaneFromTheHeader() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}}, 8, 24);
        timeline.show();
        QSignalSpy adds(&timeline, &TimelineWidget::addLaneRequested);
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QVERIFY(!timeline.addLaneVisible());
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.addLaneButtonCenter());
        QCOMPARE(adds.count(), 1);
        QVERIFY(!timeline.addLaneVisible());
        QCOMPARE(seeks.count(), 0);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "audio"}, {"clips", QJsonArray{}}}}}}, 8, 24);
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QCOMPARE(timeline.addLaneButtonCenter().y(), kRulerHeight + kTrackHeight + (kTrackHeight - 1) / 2);
    }

    void timelineHeaderShowsVideoPreview() {
        MediaArtwork artwork;
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setArtwork(&artwork);
        const QJsonObject clip{{"sourceId", "sample"}, {"timelineStart", 0}, {"duration", 2}};
        const QJsonObject track{{"type", "video"}, {"clips", QJsonArray{clip}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{track}}}, 4, 24);
        timeline.show();
        const QPoint thumbnail(kTimelineLabelWidth - 9 - kHeaderPreviewWidth / 2, kRulerHeight + kTrackHeight / 2);
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#1f1f1f"));
        QImage frame(160, 90, QImage::Format_RGB32);
        frame.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("sample"), frame);
        emit artwork.changed();
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#e87342"));
    }

    void timelineStacksPictureTracksUpwardFromV1() {
        TimelineWidget timeline;
        timeline.resize(700, 400);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{
            QJsonObject{{"id", "base"}, {"type", "video"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "dial"}, {"type", "audio"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "top"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "title"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}},
            QJsonObject{{"id", "music"}, {"type", "audio"}, {"clips", QJsonArray{}}},
        }}}, 8, 24);
        timeline.show();
        QCOMPARE(timeline.trackRowCount(), 4);
        QCOMPARE(timeline.trackRowLabel(0), QStringLiteral("V2"));
        QCOMPARE(timeline.trackRowLabel(1), QStringLiteral("V1"));
        QCOMPARE(timeline.trackRowLabel(2), QStringLiteral("A1"));
        QCOMPARE(timeline.trackRowLabel(3), QStringLiteral("A2"));
        // Playback draws the later picture track on top, so it is the upper row.
        QSignalSpy selections(&timeline, &TimelineWidget::clipSelected);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier,
                          QPoint(kTimelineLabelWidth + 100, timeline.trackHeaderRect(0).center().y()));
        QCOMPARE(selections.count(), 1);
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("title"));
    }

    void timelinePinsHeadersAndRulerWhileScrolling() {
        QScrollArea scroll;
        scroll.resize(640, 260);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        QJsonArray tracks;
        for (int i = 0; i < 8; ++i)
            tracks.append(QJsonObject{{"id", QString::number(i)}, {"type", "video"}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 120, 24);
        scroll.show();
        QTest::qWait(20);
        scroll.horizontalScrollBar()->setValue(2000);
        scroll.verticalScrollBar()->setValue(150);
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->value(), 2000);
        QCOMPARE(scroll.verticalScrollBar()->value(), 150);
        const QImage view = scroll.viewport()->grab().toImage();
        QCOMPARE(view.pixelColor(4, 4), QColor("#1a1a1a"));
        QCOMPARE(view.pixelColor(6, 60), QColor("#1f1f1f"));
        QCOMPARE(view.pixelColor(kTimelineLabelWidth + 50, kRulerHeight - 1), QColor("#0a0a0a"));

        QSignalSpy seeks(timeline, &TimelineWidget::seekRequested);
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QCOMPARE(seeks.count(), 0);
        QTest::mouseMove(timeline, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QVERIFY(timeline->skimLabel().isEmpty());
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier,
                          timeline->mapFrom(scroll.viewport(), QPoint(kTimelineLabelWidth + 72, 10)));
        QVERIFY(!seeks.isEmpty());
        QVERIFY(std::abs(seeks.constLast()[0].toDouble() - 2072.0 / 72.0) < 0.05);
    }

    void timelineTrackHeightsFollowTheHeaderEdge() {
        TimelineWidget timeline;
        timeline.resize(700, 400);
        const QJsonObject project{{"tracks", QJsonArray{
            QJsonObject{{"id", "v"}, {"type", "video"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "a"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}};
        timeline.setProject(project, 8, 24);
        timeline.show();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
        int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge));
        QTest::mouseMove(&timeline, QPoint(40, edge + 30));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge + 30));
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).top(), kRulerHeight + kVideoTrackHeight + 30);
        edge = timeline.trackHeaderRect(1).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge));
        QTest::mouseMove(&timeline, QPoint(40, 0));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, 0));
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        // Heights belong to the track, so a new snapshot keeps them.
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        timeline.fitTracksToView();
        QCOMPARE(timeline.trackHeaderRect(0).height(), timeline.trackHeaderRect(1).height());
        QVERIFY(timeline.trackHeaderRect(0).height() > kVideoTrackHeight);
        QVERIFY(timeline.trackHeaderRect(1).bottom() + kTrackHeight + 8 <= timeline.height());
        timeline.resetTrackHeights();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
    }

    void timelineScrubsFromThePointer() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy hovers(&timeline, &TimelineWidget::hoverSeekRequested);
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 144, kTrackHeight / 2));
        QVERIFY(!hovers.isEmpty());
        QCOMPARE(hovers.constLast()[0].toDouble(), 2.0);
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:02.00"));
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 36, kTrackHeight / 2));
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:00.50"));
        QCOMPARE(hovers.constLast()[0].toDouble(), 0.5);
        QTest::mouseMove(&timeline, QPoint(20, kTrackHeight / 2));
        QVERIFY(timeline.skimLabel().isEmpty());
    }

    void laneCacheBlitKeepsLogicalPixelsAtRetinaScale() {
        constexpr qreal dpr = 2;
        constexpr int width = 480;
        constexpr int height = 90;
        QPixmap cache(width * dpr, height * dpr);
        cache.setDevicePixelRatio(dpr);
        cache.fill(QColor("#141414"));
        {
            QPainter painter(&cache);
            painter.fillRect(QRect(0, 0, width, kRulerHeight), QColor("#2a2a2a"));
            painter.fillRect(QRect(120, 40, 40, 16), QColor("#3d7eff"));
        }

        const auto at = [](const QImage &image, int x, int y) {
            return image.pixelColor(QPoint(x * int(dpr), y * int(dpr)));
        };
        QImage destination(width * dpr, height * dpr, QImage::Format_ARGB32_Premultiplied);
        destination.setDevicePixelRatio(dpr);
        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(0, 0, width, height)));
        }
        QCOMPARE(at(destination, width - 8, 6).red(), 0x2a);
        QCOMPARE(at(destination, 140, 48).blue(), 0xff);

        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(300, 0, 100, height)));
        }
        QCOMPARE(at(destination, 340, 6).red(), 0x2a);
        QCOMPARE(at(destination, 20, 6).red(), 255);
    }

    void timelinePointerTimeAdvancesEveryFewPixels() {
        TimelineWidget timeline;
        timeline.resize(900, 280);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{}}}}}}, 90, 24);
        timeline.fitToWidth(900);
        timeline.show();
        QTest::qWait(20);
        const auto seconds = [](const QString &label) {
            const QStringList clock = label.split(QLatin1Char(':'));
            const QStringList sec = clock.value(1).split(QLatin1Char('.'));
            return clock.value(0).toDouble() * 60.0 + sec.value(0).toDouble() + sec.value(1).toDouble() / 100.0;
        };
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 160, y));
        const double start = seconds(timeline.skimLabel());
        QTest::mouseMove(&timeline, QPoint(kTimelineLabelWidth + 168, y));
        const double next = seconds(timeline.skimLabel());
        QVERIFY(std::abs((next - start) - 0.1) < 0.02);
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            timeline.setProject(QJsonObject{{"tracks", QJsonArray{
                QJsonObject{{"type", "video"}, {"name", "Video 1"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "opening"}, {"name", "Interview"}, {"timelineStart", 0.4}, {"duration", 3.2}},
                    QJsonObject{{"id", "broll"}, {"name", "B-roll"}, {"timelineStart", 3.8}, {"duration", 2.6}},
                }}},
                QJsonObject{{"type", "audio"}, {"name", "Dialogue"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "dial"}, {"name", "Dialogue"}, {"timelineStart", 0.2}, {"duration", 7.2}},
                }}},
            }}}, 8, 24);
            timeline.fitToWidth(980);
            timeline.resize(980, 220);
            timeline.setSelectedClip(QStringLiteral("opening"));
            timeline.setPlayhead(3.4);
            timeline.showSkimmer(5.75);
            QTest::qWait(20);
            QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
        }
    }

    void artworkDecodesMediaWithoutBlocking() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        const QString path = directory.filePath(QStringLiteral("tone.wav"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {"-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.2", path}), 0);
        MediaArtwork artwork;
        artwork.request(QJsonArray{QJsonObject{{"id", "tone"}, {"path", path}, {"hasAudio", true}}}, directory.path());
        QTRY_VERIFY_WITH_TIMEOUT(artwork.waves.contains(QStringLiteral("tone")), 5000);
        QVERIFY(!artwork.waves.value(QStringLiteral("tone")).isNull());
        QVERIFY(!artwork.frames.contains(QStringLiteral("tone")));
    }

    void selectedBoxTracksMotionAndStopsAtSceneCut() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        for (int index = 0; index < 8; ++index) {
            QImage frame(160, 90, QImage::Format_RGB888);
            frame.fill(index < 4 ? QColor(30, 30, 30) : QColor(30, 30, 220));
            if (index < 4) {
                for (int row = 0; row < 24; ++row)
                    for (int column = 0; column < 24; ++column)
                        frame.setPixelColor(16 + index * 3 + column, 20 + row,
                                            QColor(60 + column * 7, 40 + row * 7, 100 + (column + row) % 8 * 12));
            }
            QVERIFY(frame.save(directory.filePath(QStringLiteral("frame-%1.png").arg(index, 2, 10, QLatin1Char('0')))));
        }
        const QString video = directory.filePath(QStringLiteral("moving.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-framerate"), QStringLiteral("2"),
            QStringLiteral("-i"), directory.filePath(QStringLiteral("frame-%02d.png")),
            QStringLiteral("-c:v"), QStringLiteral("libx264"), QStringLiteral("-pix_fmt"),
            QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        MainWindow window{directory.path(), QString()};
        window.m_fps = 10;
        window.m_project = QJsonObject{{QStringLiteral("sources"), QJsonArray{
            QJsonObject{{QStringLiteral("id"), QStringLiteral("moving")}, {QStringLiteral("path"), video}}}}};
        const QJsonObject clip{{QStringLiteral("sourceId"), QStringLiteral("moving")},
                               {QStringLiteral("timelineStart"), 0}, {QStringLiteral("in"), 0},
                               {QStringLiteral("out"), 4}, {QStringLiteral("speed"), 1}};
        const QRectF seed(16.0 / 160, 20.0 / 90, 24.0 / 160, 24.0 / 90);
        QVERIFY(!window.sourceFrame(clip, 0).isNull());
        const double cut = window.firstSceneCut(clip, 0, 4);
        QVERIFY2(std::abs(cut - 2.0) < 0.1, qPrintable(QString::number(cut)));
        double reliableEnd = 4;
        const auto boxes = window.trackRegion(clip, 0, 4, seed, &reliableEnd);
        QVERIFY(boxes.size() >= 2);
        QVERIFY(boxes.last().second.x() > seed.x());
        QVERIFY(reliableEnd <= 2.5);
    }

    void livePreviewPlaysAndCanStillRender() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "required"}});
        QVERIFY(window.m_previewRequired);
        // The compositor draws the program, so it plays; its note says what
        // only a render matches.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "live"},
                                       {"message", "`a1` uses a crossfade, whose sound only a render plays exactly"}});
        QVERIFY(!window.m_previewRequired);
        QCOMPARE(window.m_previewStatus, QStringLiteral("live"));
        QVERIFY(!window.m_statusDevice->isHidden());
        QCOMPARE(window.m_statusDevice->text(), QStringLiteral("Live"));
        QVERIFY(window.statusBar()->toolTip().contains(QStringLiteral("crossfade")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("sequence.renderPreview")), QString());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(!window.m_commands->reason(QStringLiteral("sequence.renderPreview")).isEmpty());
        QVERIFY(window.m_statusDevice->isHidden());
    }

    void disconnectedControlsAndRecovery() {
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("CONNECTING"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_timeline->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("OFFLINE"));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_connectionBanner->isHidden());
        QVERIFY(window.m_timeline->isEnabled());
        window.m_chatInput->setText(QStringLiteral("테스트 편집"));
        QVERIFY(window.m_sendButton->isEnabled());
        const auto conversation = window.m_conversation->toPlainText();
        const int activityCount = window.m_activityList->count();
        window.showError(QStringLiteral("Render failed"));
        window.showError(QStringLiteral("Render failed"));
        QCOMPARE(window.m_conversation->toPlainText(), conversation);
        QCOMPARE(window.m_activityList->count(), activityCount);
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Render failed"));
    }

    void timelineFitsAndAllTracksRemainReachable() {
        QScrollArea scroll;
        scroll.resize(640, 230);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        scroll.show();
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->maximum(), 0);
        QJsonArray tracks;
        for (int i = 0; i < 12; ++i) tracks.append(QJsonObject{{"id", QString::number(i)}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 3600, 24);
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        // The vertical scrollbar changes the available width.
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        // An hour stays at least 80px per second, so the pointer can address
        // frames instead of collapsing onto a handful of pixels.
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
        QVERIFY(scroll.verticalScrollBar()->maximum() > 0);
        QVERIFY(timeline->minimumHeight() >= kRulerHeight + 12 * kTrackHeight);
        timeline->setZoom(1.0);
        QTest::qWait(20);
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
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
        QCOMPARE(window.m_proposalSummary->text(), QStringLiteral("2 changed lines"));
        QVERIFY(window.m_acceptButton->isEnabled());
        proposal.insert(QStringLiteral("freshness"), QStringLiteral("stale"));
        window.updateProposal(proposal);
        QVERIFY(!window.m_acceptButton->isEnabled());
        QVERIFY(window.m_proposalDiff->toPlainText().contains(QStringLiteral("out of date")));
    }

    // A `collab` status for a shared project on main.
    static QJsonObject sharedStatus() {
        return QJsonObject{{"repository", true}, {"branch", "main"}, {"upstream", "origin/main"},
                           {"remote", "git@github.com:jun/film.git"}, {"ahead", 2}, {"behind", 1}, {"changed", false},
                           {"conflicted", false}, {"branches", QJsonArray{"main", "color"}}, {"lfs", true},
                           {"gh", true}, {"live", false}, {"busy", false}};
    }

    // Answer the next modal QInputDialog with `text`.
    static void answerInput(QWidget *parent, const QString &text) {
        QTimer::singleShot(0, parent, [parent, text] {
            auto *dialog = parent->findChild<QInputDialog *>();
            if (!dialog) {
                if (QWidget *modal = QApplication::activeModalWidget()) modal->close();
                QFAIL("no input dialog");
            }
            dialog->setTextValue(text);
            dialog->accept();
        });
    }

    void syncChipFollowsTheCollabStatus() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(window.m_syncChip->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Reading the project's Git status"));
        const QJsonObject status = sharedStatus();
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QVERIFY(!window.m_syncChip->isHidden());
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⎇ main ↑2 ↓1"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("idle"));
        QVERIFY(window.m_syncChip->toolTip().contains(QStringLiteral("Syncs with origin/main")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.commit")), QStringLiteral("Nothing to commit"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.publish")), QStringLiteral("The project is already published"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));

        // A click on the chip syncs now.
        window.m_lastCommand = {};
        window.m_syncChip->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_sync"}}));

        // Switch Branch lists the other branches.
        window.buildBranchMenu(window.m_branchMenu);
        QCOMPARE(window.m_branchMenu->actions().size(), 1);
        window.m_branchMenu->actions().first()->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_branch"}, {"name", "color"}, {"create", false}}));

        // While a sync runs the chip says what it is doing, and Sync Now waits.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}});
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⟳ Pulling…"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("syncing"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("A sync is running"));

        // A finished sync lists what came in and what kept yours, then
        // reads the preset library again.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{QJsonObject{{"commit", "a1b2c3d"}, {"author", "jun"}, {"subject", "Trimmed c3"}}}},
            {"fastForward", false}, {"pushed", true},
            {"conflicts", QJsonArray{QJsonObject{{"path", "project.kerf"}, {"what", "c3 duration"}, {"kept", "ours"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{"Trimmed c3"}}}}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("idle"));
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("jun · Trimmed c3")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept yours · project.kerf: c3 duration")));
        QVERIFY(window.m_syncChip->toolTip().contains(QStringLiteral("jun · Trimmed c3")));
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Synced: 1 commit in, pushed, 1 change kept yours"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("preset_library"));

        // A failure goes to the error banner.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "the remote rejected the push"}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("failed"));
        QVERIFY(window.m_syncChip->text().endsWith(QStringLiteral("sync failed")));
        QVERIFY(!window.m_errorBanner->isHidden());
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Sync failed: the remote rejected the push"));

        QJsonObject conflicted = status;
        conflicted.insert("conflicted", true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", conflicted}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("conflict"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Finish the merge in progress with git first"));

        const QJsonObject local{{"repository", false}, {"branches", QJsonArray{}}, {"gh", false}, {"live", false}, {"busy", false}};
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", local}});
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⎇ Not shared"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("local"));
        QVERIFY(window.m_commands->reason(QStringLiteral("collab.sync")).startsWith(QStringLiteral("Share the project with Git first")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.publish")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.pullRequest")), window.m_commands->reason(QStringLiteral("collab.sync")));
    }

    void liveSyncAndCommitsAreCollabRequests() {
        QSettings().remove(QStringLiteral("collab/liveProjects"));
        QTemporaryDir project;
        const QString path = QDir::cleanPath(QFileInfo(project.path()).absoluteFilePath());
        QJsonObject status = sharedStatus();
        status.insert("changed", true);
        {
            MainWindow window{project.path(), QString()};
            window.setConnection(QStringLiteral("ready"));
            // Without live sync remembered, connecting asks for the status and the library.
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            QAction *live = window.m_commands->action(QStringLiteral("collab.live"));
            QVERIFY(live->isCheckable() && live->isEnabled() && !live->isChecked());
            live->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", true}}));
            QVERIFY(QSettings().value(QStringLiteral("collab/liveProjects")).toStringList().contains(path));
            status.insert("live", true);
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("live"));
            QVERIFY(window.m_syncChip->text().endsWith(QStringLiteral("· live")));

            window.m_commands->action(QStringLiteral("collab.sync"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_sync"}}));
            answerInput(&window, QStringLiteral("Tighten the intro"));
            window.m_commands->action(QStringLiteral("collab.commit"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_commit"}, {"message", "Tighten the intro"}}));
            answerInput(&window, QStringLiteral("color-pass"));
            window.m_commands->action(QStringLiteral("collab.newBranch"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_branch"}, {"name", "color-pass"}, {"create", true}}));
        }
        {
            // The next window for the project turns live sync on as it connects.
            MainWindow window{project.path(), QString()};
            QAction *live = window.m_commands->action(QStringLiteral("collab.live"));
            QVERIFY(live->isChecked());
            window.setConnection(QStringLiteral("ready"));
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", true}}));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            live->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", false}}));
            QVERIFY(!QSettings().value(QStringLiteral("collab/liveProjects")).toStringList().contains(path));
        }
        QSettings().remove(QStringLiteral("collab/liveProjects"));
    }

    void divergedProjectionIsLoadedOrKept() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        const QJsonObject timeline{{"fps", 24}, {"tracks", QJsonArray{}}};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 0}, {"projectionStatus", "diverged"}, {"timeline", timeline}});
        QVERIFY(!window.m_divergedBanner->isHidden());
        QVERIFY(window.findChild<QLabel *>(QStringLiteral("divergedMessage"))->text().contains(QStringLiteral("project.kerf changed on disk")));
        // kerfd refuses to sync until one side is chosen.
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")),
                 QStringLiteral("project.kerf changed on disk: load it or keep the editor's version first"));
        window.m_loadFromDisk->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "projection_import"}}));
        window.m_keepEditorVersion->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "projection_repair"}}));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 0}, {"projectionStatus", "clean"}, {"timeline", timeline}});
        QVERIFY(window.m_divergedBanner->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("projection.import")), QStringLiteral("project.kerf matches the editor"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
    }

    void presetsPanelListsAppliesAndSavesTheLibrary() {
        QSettings().remove(QStringLiteral("workspace"));
        QTemporaryDir project;
        MainWindow window{project.path(), QString()};
        // The Editing layout keeps Presets behind the Project bin.
        QVERIFY(window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        QVERIFY(!window.m_workspace->panel(QStringLiteral("presets"))->isClosed());
        PresetsPanel *panel = window.m_presets;
        QVERIFY(!panel->m_apply->isEnabled());
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
        window.handleEvent(QJsonObject{{"type", "preset_library"}, {"directory", "presets"}, {"presets", QJsonArray{
            QJsonObject{{"path", "presets/intro.kerfpreset"}, {"name", "Intro"}, {"author", "jun"}, {"updated", "2026-10-30T09:00:00Z"},
                        {"slots", QJsonArray{
                            QJsonObject{{"id", "slot1"}, {"seconds", 4}, {"video", true}, {"audio", true}, {"clips", 2}},
                            QJsonObject{{"id", "slot2"}, {"seconds", 2}, {"video", true}, {"audio", false}, {"clips", 1}}}},
                        {"texts", QJsonObject{{"c4", "Title"}}}},
            QJsonObject{{"path", "presets/broken.kerfpreset"}, {"name", "broken"}, {"error", "line 3: unknown statement"}}}}});
        QCOMPARE(panel->m_list->count(), 2);
        QVERIFY(panel->m_list->item(0)->text().startsWith(QStringLiteral("Intro\n2 slots · 1 text · jun · ")));
        QVERIFY(panel->m_list->item(0)->toolTip().contains(QStringLiteral("slot1 · 4.00 s · picture and sound · 2 clips")));
        QVERIFY(panel->m_list->item(1)->text().contains(QStringLiteral("line 3: unknown statement")));
        panel->m_list->setCurrentRow(1);
        QVERIFY(!panel->m_apply->isEnabled());
        panel->m_list->setCurrentRow(0);
        QVERIFY(panel->m_apply->isEnabled());
        // Apply asks Kerf which media fits each slot, as Import Editing Preset does.
        panel->m_apply->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_inspect"},
                                                    {"path", QDir(project.path()).absoluteFilePath("presets/intro.kerfpreset")}}));
        answerInput(&window, QStringLiteral("Teaser"));
        panel->m_save->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_save"}, {"name", "Teaser"}, {"replace", false}}));
        window.handleEvent(QJsonObject{{"type", "preset_saved"}, {"path", "presets/Teaser.kerfpreset"}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
        window.handleEvent(QJsonObject{{"type", "preset_library"}, {"directory", "presets"}, {"presets", QJsonArray{}}});
        QVERIFY(panel->m_list->isHidden());
        QVERIFY(!panel->m_empty->isHidden());
        QSettings().remove(QStringLiteral("workspace"));
    }

    void projectBinShowsMediaStatusAndReplacesMedia() {
        QTemporaryDir project;
        QVERIFY(QDir(project.path()).mkpath(QStringLiteral("media")));
        QFile file(QDir(project.path()).filePath(QStringLiteral("media/a.mov")));
        QVERIFY(file.open(QIODevice::WriteOnly));
        file.write("frames");
        file.close();
        MainWindow window{project.path(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{}}, {"sources", QJsonArray{
                QJsonObject{{"id", "m1"}, {"path", "media/a.mov"}, {"width", 320}, {"duration", 4}},
                QJsonObject{{"id", "m2"}, {"path", "/Volumes/Shoot/b.mov"}, {"width", 320}, {"duration", 4}},
                QJsonObject{{"id", "m3"}, {"path", "media/c.mov"}, {"width", 320}, {"duration", 4}}}}}}});
        window.handleEvent(QJsonObject{{"type", "media_status"}, {"missing", QJsonArray{"m3"}},
                                       {"pointers", QJsonArray{"m1"}}, {"external", QJsonArray{"m2"}}});
        ProjectPanel *bin = window.m_projectPanel;
        QCOMPARE(bin->badge(QStringLiteral("m1")), QStringLiteral("NOT DOWNLOADED"));
        QCOMPARE(bin->badge(QStringLiteral("m2")), QStringLiteral("EXTERNAL"));
        QCOMPARE(bin->badge(QStringLiteral("m3")), QStringLiteral("OFFLINE"));
        QVERIFY(bin->m_list->item(1)->toolTip().contains(QStringLiteral("Consolidate Media")));
        // Consolidating waits for the Git status, then for media outside the project.
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.consolidate")), QStringLiteral("Reading the project's Git status"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.consolidate")));

        QMenu *menu = window.binMenu(QStringLiteral("m1"));
        QStringList labels;
        for (const QAction *action : menu->actions())
            if (!action->isSeparator()) labels << action->text();
        QCOMPARE(labels, (QStringList{"Open in Source Monitor", "Replace Media…", "Reveal in Finder", "Import Media…", "Consolidate Media"}));
        QVERIFY(menu->findChild<QAction *>(QStringLiteral("binReplace"))->isEnabled());
        QVERIFY(menu->findChild<QAction *>(QStringLiteral("binReveal"))->isEnabled());
        delete menu;
        menu = window.binMenu(QStringLiteral("m3"));
        QVERIFY(!menu->findChild<QAction *>(QStringLiteral("binReveal"))->isEnabled());
        delete menu;

        window.replaceMedia(QStringLiteral("m2"), QStringLiteral("/Volumes/Shoot/b-take2.mov"));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_replace"}, {"media", "m2"}, {"path", "/Volumes/Shoot/b-take2.mov"}}));
        window.m_commands->action(QStringLiteral("collab.consolidate"))->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_consolidate"}}));
        window.handleEvent(QJsonObject{{"type", "media_status"}, {"missing", QJsonArray{}}, {"pointers", QJsonArray{}}, {"external", QJsonArray{}}});
        QCOMPARE(bin->badge(QStringLiteral("m2")), QString());
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.consolidate")), QStringLiteral("All media is inside the project folder"));
    }

    void projectLauncherValidatesWithoutStartingTasks() {
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
        QTemporaryDir root;
        const QString location = QDir(root.path()).filePath(QStringLiteral("Projects"));
        const QString taken = QDir(location).filePath(QStringLiteral("Taken"));
        const QString old = QDir(root.path()).filePath(QStringLiteral("Old Cut"));
        QVERIFY(QDir().mkpath(QDir(taken).filePath(QStringLiteral("media"))));
        QVERIFY(QDir().mkpath(old));

        // Recent projects: most recent first, without repeats, at most twelve.
        for (int i = 0; i < 20; ++i) ProjectLauncher::rememberProject(QDir(root.path()).filePath(QStringLiteral("p%1").arg(i)));
        QCOMPARE(ProjectLauncher::recentProjects().size(), ProjectLauncher::kRecentLimit);
        QCOMPARE(ProjectLauncher::recentProjects().first(), QDir(root.path()).filePath(QStringLiteral("p19")));
        ProjectLauncher::clearRecentProjects();
        ProjectLauncher::rememberProject(old);
        ProjectLauncher::rememberProject(taken);
        ProjectLauncher::rememberProject(old);
        QCOMPARE(ProjectLauncher::recentProjects(), (QStringList{old, taken}));

        ProjectLauncher launcher(QStringLiteral("/opt/kerf/bin/kerfd"));
        // It asks for the tools; a test build starts no kerf_task_* task.
        QCOMPARE(launcher.m_requests, (QList<QJsonObject>{QJsonObject{{"type", "tools"}}}));
        QVERIFY(launcher.m_tasks.isEmpty());
        // Without an inspection the folder name stands in.
        QVERIFY(launcher.m_recent->item(0)->text().startsWith(QStringLiteral("Old Cut\n")));
        QCOMPARE(launcher.m_sidebar->count(), 4);

        launcher.showPage(ProjectLauncher::NewProject);
        QCOMPARE(launcher.m_heading->text(), QStringLiteral("Choose a template"));
        QVERIFY(!launcher.m_details->isHidden());
        QCOMPARE(launcher.m_templates->count(), 3);
        launcher.m_templates->setCurrentRow(1);
        launcher.m_location->setText(location);
        launcher.m_name->setText(QString());
        QVERIFY(!launcher.m_choose->isEnabled());
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("Name the project"));
        launcher.m_name->setText(QStringLiteral("Taken"));
        QVERIFY(!launcher.m_choose->isEnabled());
        QVERIFY(launcher.m_problem->text().contains(QStringLiteral("already in that location")));
        launcher.m_name->setText(QStringLiteral("My Film"));
        QVERIFY(launcher.m_choose->isEnabled());
        QVERIFY(launcher.m_destination->text().endsWith(QStringLiteral("Projects/My Film")));
        launcher.m_remote->setText(QStringLiteral("not an address"));
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("The remote must be a Git address"));
        QVERIFY(launcher.m_git->isChecked());
        launcher.m_remote->clear();
        launcher.m_rate->setCurrentIndex(launcher.m_rate->findData(QStringLiteral("24")));

        // A private GitHub repository needs a signed-in GitHub CLI.
        QVERIFY(!launcher.m_github->isEnabled());
        launcher.setTools(QJsonObject{{"git", true}, {"gitLfs", true}, {"gh", true}, {"ghUser", "jun"}});
        QVERIFY(launcher.m_github->isEnabled());
        QVERIFY(launcher.m_gitHint->text().contains(QStringLiteral("@jun")));
        launcher.m_github->setChecked(true);
        QVERIFY(launcher.m_git->isChecked());
        QVERIFY(!launcher.m_remote->isEnabled());
        launcher.m_choose->click();
        QCOMPARE(launcher.m_requests.last(), (QJsonObject{{"type", "create"}, {"name", "My Film"}, {"location", location},
                                                          {"template", "vertical"}, {"rate", "24"}, {"git", true},
                                                          {"github", QJsonObject{{"private", true}}},
                                                          {"kerfd", "/opt/kerf/bin/kerfd"}}));
        QVERIFY(launcher.m_busy);
        QVERIFY(!launcher.m_choose->isEnabled());
        QVERIFY(!launcher.m_progress->isHidden());
        // The bridge answers a failed task; the launcher says why and lets one try again.
        launcher.handleTaskStatus(QJsonObject{{"status", "failed"}, {"error", "not implemented yet"}});
        QVERIFY(!launcher.m_busy);
        QVERIFY(launcher.m_choose->isEnabled());
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("not implemented yet"));
        QCOMPARE(launcher.m_problem->property("state").toString(), QStringLiteral("error"));
        launcher.handleTaskStatus(QJsonObject{{"status", "done"}, {"path", QDir(location).filePath("My Film")},
                                              {"warnings", QJsonArray{"Git LFS is not installed"}}});
        QCOMPARE(launcher.result(), int(QDialog::Accepted));
        QCOMPARE(launcher.chosenPath(), QDir(location).filePath(QStringLiteral("My Film")));

        // Recent projects, with what kerf_project_inspect says of them.
        ProjectLauncher recent;
        recent.m_inspectProject = [old](const QJsonObject &request) {
            const QString path = request.value("path").toString();
            if (path == old) return QJsonObject{{"path", path}, {"exists", false}};
            return QJsonObject{{"path", path}, {"exists", true}, {"project", true}, {"name", "Taken Film"},
                               {"branch", "main"}, {"presets", 2}};
        };
        recent.reloadRecent();
        recent.showPage(ProjectLauncher::Recent);
        QVERIFY(recent.m_details->isHidden());
        QCOMPARE(recent.m_recent->count(), 2);
        QVERIFY(recent.m_recent->item(1)->text().startsWith(QStringLiteral("Taken Film\n")));
        QVERIFY(recent.m_recent->item(1)->text().contains(QStringLiteral("⎇ main")));
        QVERIFY(recent.m_recent->item(1)->text().contains(QStringLiteral("2 presets")));
        recent.m_recent->setCurrentRow(0);
        QVERIFY(!recent.m_choose->isEnabled());
        QCOMPARE(recent.m_problem->text(), QStringLiteral("That project folder is no longer there"));
        recent.m_recent->setCurrentRow(1);
        recent.m_choose->click();
        QCOMPARE(recent.result(), int(QDialog::Accepted));
        QCOMPARE(recent.chosenPath(), taken);
        QCOMPARE(recent.m_requests.size(), 1);

        // Clone from a Git address into a free folder.
        ProjectLauncher clone;
        clone.showPage(ProjectLauncher::Clone);
        clone.m_cloneLocation->setText(location);
        clone.m_cloneUrl->setText(QStringLiteral("film"));
        QVERIFY(!clone.m_choose->isEnabled());
        QCOMPARE(clone.m_problem->text(), QStringLiteral("Enter a Git address such as https://github.com/you/film.git"));
        clone.m_cloneUrl->setText(QStringLiteral("git@github.com:jun/Taken.git"));
        QVERIFY(clone.m_problem->text().contains(QStringLiteral("already in that location")));
        clone.m_cloneUrl->setText(QStringLiteral("https://github.com/jun/film.git"));
        QVERIFY(clone.m_choose->isEnabled());
        QVERIFY(clone.m_cloneDestination->text().endsWith(QStringLiteral("Projects/film")));
        clone.m_choose->click();
        QCOMPARE(clone.m_requests.last(), (QJsonObject{{"type", "clone"}, {"url", "https://github.com/jun/film.git"}, {"location", location}}));

        // From a preset: each slot needs a file that fits it.
        ProjectLauncher fromPreset;
        fromPreset.m_inspectPreset = [](const QJsonObject &request) {
            if (!request.contains("files"))
                return QJsonObject{{"name", "Intro"}, {"texts", QJsonObject{{"c4", "Title"}}}, {"slots", QJsonArray{
                    QJsonObject{{"id", "slot1"}, {"seconds", 4}, {"video", true}, {"audio", true}, {"clips", 2}},
                    QJsonObject{{"id", "slot2"}, {"seconds", 1}, {"video", true}, {"audio", false}, {"clips", 1}}}}};
            QJsonArray files;
            for (const QJsonValue &file : request.value("files").toArray()) {
                const bool longer = file.toString().endsWith(QStringLiteral("long.mov"));
                files.append(QJsonObject{{"path", file}, {"seconds", longer ? 10 : 2}, {"video", true}, {"audio", true},
                                         {"fits", longer ? QJsonArray{"slot1", "slot2"} : QJsonArray{"slot2"}}});
            }
            return QJsonObject{{"files", files}};
        };
        fromPreset.showPage(ProjectLauncher::FromPreset);
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("Choose a preset"));
        fromPreset.m_name->setText(QStringLiteral("Teaser"));
        fromPreset.m_location->setText(location);
        fromPreset.loadPreset(QStringLiteral("/library/intro.kerfpreset"));
        QVERIFY(fromPreset.findChild<QPushButton *>(QStringLiteral("presetSlotFile_slot1")));
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("Choose a file for each media slot"));
        fromPreset.setSlotFile(QStringLiteral("slot1"), QStringLiteral("/clips/short.mov"));
        fromPreset.setSlotFile(QStringLiteral("slot2"), QStringLiteral("/clips/short.mov"));
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("short.mov cannot fill slot1"));
        auto *status = fromPreset.findChild<QLabel *>(QStringLiteral("presetSlotStatus_slot1"));
        QVERIFY(status->text().contains(QStringLiteral("is shorter than 4.00 s")));
        QCOMPARE(status->property("fit").toString(), QStringLiteral("no"));
        auto *start = fromPreset.findChild<QDoubleSpinBox *>(QStringLiteral("presetSlotFrom_slot1"));
        QVERIFY(!start->isEnabled());
        fromPreset.setSlotFile(QStringLiteral("slot1"), QStringLiteral("/clips/long.mov"));
        QVERIFY(status->text().startsWith(QStringLiteral("✓ long.mov")));
        QVERIFY(fromPreset.m_choose->isEnabled());
        // The slot can start later in a longer file.
        QVERIFY(start->isEnabled());
        QCOMPARE(start->maximum(), 6.0);
        start->setValue(1.5);
        auto *title = fromPreset.findChild<QLineEdit *>(QStringLiteral("presetText_c4"));
        title->setText(QStringLiteral("New title"));
        title->setModified(true);
        fromPreset.m_choose->click();
        const QJsonObject request = fromPreset.m_requests.last();
        QCOMPARE(request.value("type").toString(), QStringLiteral("create"));
        QCOMPARE(request.value("name").toString(), QStringLiteral("Teaser"));
        QCOMPARE(request.value("preset").toObject(), (QJsonObject{
            {"path", "/library/intro.kerfpreset"},
            {"bindings", QJsonObject{{"slot1", QJsonObject{{"file", "/clips/long.mov"}, {"from", "1.50s"}}},
                                     {"slot2", QJsonObject{{"file", "/clips/short.mov"}}}}},
            {"texts", QJsonObject{{"c4", "New title"}}}}));

        // File ▸ Open Recent opens a project in a new window.
        MainWindow window{taken, QString()};
        window.buildRecentMenu(window.m_recentMenu);
        QAction *openOld = nullptr;
        for (QAction *action : window.m_recentMenu->actions())
            if (action->text() == QStringLiteral("Old Cut")) openOld = action;
        QVERIFY(openOld);
        QSignalSpy opened(&window, &MainWindow::projectOpenRequested);
        openOld->trigger();
        QCOMPARE(opened.count(), 1);
        QCOMPARE(opened.first().first().toString(), old);

        ProjectLauncher::clearRecentProjects();
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
    }

    // The launcher follows the system appearance with its own style sheet,
    // whatever the editor's: every page in light and in dark. With
    // KERF_SCREENSHOT_DIR set, each is saved as <page>-<scheme>.png.
    void launcherRendersBothAppearances() {
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
        QTemporaryDir root;
        QVERIFY(root.isValid());
        QStringList projects;
        for (const char *name : {"Harbor Teaser", "Night Market", "Studio Session"}) {
            projects << QDir(root.path()).filePath(QString::fromLatin1(name));
            QVERIFY(QDir().mkpath(QDir(projects.last()).filePath(QStringLiteral("presets"))));
        }
        QFile presetFile(QDir(projects.first()).filePath(QStringLiteral("presets/Intro Titles.kerfpreset")));
        QVERIFY(presetFile.open(QIODevice::WriteOnly));
        presetFile.write("kerf 1\n");
        presetFile.close();
        ProjectLauncher::clearRecentProjects();
        for (auto it = projects.crbegin(); it != projects.crend(); ++it) ProjectLauncher::rememberProject(*it);
        const auto inspectProject = [projects](const QJsonObject &request) {
            const QString path = request.value("path").toString();
            const int index = std::max(0, int(projects.indexOf(path)));
            static const char *const branches[] = {"main", "color-pass", "main"};
            static const char *const edited[] = {"2026-10-30T18:42:00Z", "2026-10-27T09:15:00Z", "2026-10-21T21:03:00Z"};
            return QJsonObject{{"path", path}, {"exists", true}, {"project", true}, {"name", QFileInfo(path).fileName()},
                               {"branch", branches[index]}, {"modified", edited[index]}, {"presets", index == 0 ? 1 : 0}};
        };
        const auto inspectPreset = [](const QJsonObject &request) {
            if (!request.contains("files"))
                return QJsonObject{{"name", "Intro Titles"}, {"texts", QJsonObject{{"c4", "Harbor Teaser"}}}, {"slots", QJsonArray{
                    QJsonObject{{"id", "Opening"}, {"seconds", 2}, {"video", true}, {"audio", true}, {"clips", 2}},
                    QJsonObject{{"id", "Logo"}, {"seconds", 1.5}, {"video", true}, {"audio", false}, {"clips", 1}}}}};
            QJsonArray files;
            for (const QJsonValue &file : request.value("files").toArray())
                files.append(QJsonObject{{"path", file}, {"seconds", 6}, {"video", true}, {"audio", true},
                                         {"fits", QJsonArray{"Opening", "Logo"}}});
            return QJsonObject{{"files", files}};
        };
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        // Nothing amber or gold is left from the editor's palette.
        const auto amberPixels = [](const QImage &image) {
            int count = 0;
            for (int y = 0; y < image.height(); y += 2)
                for (int x = 0; x < image.width(); x += 2) {
                    const QColor color = image.pixelColor(x, y);
                    if (color.hue() >= 30 && color.hue() <= 52 && color.saturation() > 140 && color.value() > 150) ++count;
                }
            return count;
        };
        const char *const pageNames[] = {"new", "recent", "preset", "clone"};
        for (const auto scheme : {ProjectLauncher::ColorScheme::Light, ProjectLauncher::ColorScheme::Dark}) {
            const bool dark = scheme == ProjectLauncher::ColorScheme::Dark;
            const QString schemeName = dark ? QStringLiteral("dark") : QStringLiteral("light");
            for (int page = ProjectLauncher::NewProject; page <= ProjectLauncher::Clone; ++page) {
                QSettings().remove(QStringLiteral("launcher"));
                ProjectLauncher launcher;
                launcher.setColorScheme(scheme);
                QCOMPARE(launcher.theme().dark, dark);
                launcher.m_inspectProject = inspectProject;
                launcher.m_inspectPreset = inspectPreset;
                launcher.reloadRecent();
                launcher.reloadPresetChoices();
                launcher.setTools(QJsonObject{{"git", true}, {"gitLfs", true}, {"gh", true}, {"ghUser", "jun"}});
                QCOMPARE(launcher.m_recent->count(), 3);
                launcher.showPage(static_cast<ProjectLauncher::Page>(page));
                if (page == ProjectLauncher::FromPreset) {
                    launcher.m_presetChoices->setCurrentRow(0);
                    QCOMPARE(launcher.m_slotOrder, (QStringList{"Opening", "Logo"}));
                    launcher.setSlotFile(QStringLiteral("Opening"), QStringLiteral("/Volumes/Shoot/harbor-wide.mov"));
                } else if (page == ProjectLauncher::Clone) {
                    launcher.m_cloneUrl->setText(QStringLiteral("https://github.com/jun/harbor-teaser.git"));
                }
                launcher.resize(1040, 650);
                launcher.show();
                QVERIFY(QTest::qWaitForWindowExposed(&launcher));
                // A chosen recent project is blue while its list has focus.
                if (page == ProjectLauncher::Recent) launcher.m_recent->setFocus();
                QCoreApplication::processEvents();
                QImage image = launcher.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const auto at = [&image, ratio](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio).rgb(); };
                QCOMPARE(at(launcher.width() - 6, 6), launcher.theme().content.rgb());
                QCOMPARE(at(100, launcher.height() - 10), launcher.theme().sidebar.rgb());
                QVERIFY(dark ? launcher.theme().content.lightness() < 60 : launcher.theme().content.lightness() > 240);
                if (launcher.m_choose->isEnabled()) {
                    // The default button is the accent, lightening towards its top.
                    const QImage choose = launcher.m_choose->grab().toImage();
                    const QColor fill = choose.pixelColor(QPoint(8, launcher.m_choose->height() / 2) * choose.devicePixelRatio());
                    const QColor accent = launcher.theme().accent;
                    QVERIFY2(std::abs(fill.hue() - accent.hue()) <= 4 && std::abs(fill.value() - accent.value()) <= 24,
                             qPrintable(fill.name()));
                }
                QCOMPARE(amberPixels(image), 0);
#if defined(Q_OS_MACOS) && QT_VERSION >= QT_VERSION_CHECK(6, 9, 0)
                // The sidebar runs up under a transparent title bar; the content titles itself.
                QVERIFY(launcher.windowFlags().testFlag(Qt::ExpandedClientAreaHint));
                QCOMPARE(launcher.m_title->text(), launcher.windowTitle());
                QCOMPARE(at(launcher.width() - 6, launcher.m_title->height() - 1), launcher.theme().titleRule.rgb());
#endif
                save(image, QStringLiteral("%1-%2").arg(QString::fromLatin1(pageNames[page]), schemeName));
                if (page == ProjectLauncher::NewProject) {
                    // Details are folded away until opened, as in macOS.
                    QVERIFY(launcher.m_detailsBody->isHidden());
                    launcher.m_detailsToggle->click();
                    QVERIFY(!launcher.m_detailsBody->isHidden());
                    launcher.m_git->setChecked(true);
                    launcher.m_name->setFocus();
                    QCoreApplication::processEvents();
                    image = launcher.grab().toImage();
                    QCOMPARE(amberPixels(image), 0);
                    save(image, QStringLiteral("new-details-%1").arg(schemeName));
                }
                launcher.close();
            }
        }
        ProjectLauncher::clearRecentProjects();
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
    }

    // Saturated amber or gold, as the editor's old accent was.
    static bool isAmber(const QColor &color) {
        return color.hue() >= 30 && color.hue() <= 52 && color.saturation() > 140 && color.value() > 150;
    }
    static int amberPixels(const QImage &image) {
        int count = 0;
        for (int y = 0; y < image.height(); y += 2)
            for (int x = 0; x < image.width(); x += 2)
                if (isAmber(image.pixelColor(x, y))) ++count;
        return count;
    }
    // The colour most of `image` is.
    static QRgb dominantColor(const QImage &image) {
        QHash<QRgb, int> counts;
        for (int y = 0; y < image.height(); y += 2)
            for (int x = 0; x < image.width(); x += 2) ++counts[image.pixel(x, y)];
        QRgb best = 0;
        int most = 0;
        for (auto it = counts.cbegin(); it != counts.cend(); ++it)
            if (it.value() > most) { most = it.value(); best = it.key(); }
        return best;
    }

    // A cut with every kind of clip: two picture tracks (one of titles), a
    // caption track, dialogue and music, a marker, and a pending proposal
    // that trims one clip, drops another, and adds a title.
    static QJsonObject showcaseSnapshot(int revision, const QString &projection) {
        const auto clip = [](const char *id, const char *source, double start, double duration, double in = 0.0) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", source}, {"timelineStart", start},
                               {"duration", duration}, {"in", in}, {"out", in + duration}};
        };
        QJsonObject graded = clip("v1c", "drone", 11.0, 7.0);
        graded.insert("effects", QJsonArray{QJsonObject{{"fx", "color.grade"}, {"version", 1}, {"enabled", true}, {"params", QJsonObject{}}}});
        const QJsonArray sources{
            QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}, {"hasAudio", true}},
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}, {"hasAudio", true}},
            QJsonObject{{"id", "drone"}, {"path", "media/drone-pass.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}, {"hasAudio", true}}};
        const QJsonObject title{{"id", "t1"}, {"kind", "generator"}, {"fx", "title"}, {"name", "Harbor Teaser"},
                                {"timelineStart", 1.0}, {"duration", 4.0}};
        const auto caption = [](const char *id, const char *text, double start, double duration) {
            return QJsonObject{{"id", id}, {"kind", "caption"}, {"text", text}, {"name", text}, {"timelineStart", start}, {"duration", duration}};
        };
        const auto timeline = [&](bool proposed) {
            QJsonObject v1b = clip("v1b", "close", 6.0, proposed ? 3.5 : 5.0, 2.0);
            QJsonArray v1{clip("v1a", "wide", 0.0, 6.0), v1b};
            if (!proposed) v1.append(graded);
            QJsonArray titles{title};
            if (proposed) titles.append(QJsonObject{{"id", "t2"}, {"kind", "generator"}, {"fx", "title"}, {"name", "Lower third"},
                                                    {"timelineStart", 12.0}, {"duration", 3.0}});
            return QJsonObject{
                {"name", "Harbor Teaser"}, {"fps", 24}, {"width", 1920}, {"height", 1080}, {"sources", sources},
                {"markers", QJsonArray{QJsonObject{{"id", "m1"}, {"time", 6.0}, {"duration", 0.0}, {"color", "red"}},
                                       QJsonObject{{"id", "m2"}, {"time", 13.0}, {"duration", 2.0}, {"color", "blue"}}}},
                {"tracks", QJsonArray{
                    QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", v1}},
                    QJsonObject{{"id", "V2"}, {"type", "video"}, {"name", "Titles"}, {"clips", titles}},
                    QJsonObject{{"id", "S1"}, {"type", "subtitle"}, {"name", "Captions"}, {"clips", QJsonArray{
                        caption("c1", "We open on the harbor at dawn.", 2.0, 4.0),
                        caption("c2", "Every boat has a name.", 9.0, 4.0)}}},
                    QJsonObject{{"id", "A1"}, {"type", "audio"}, {"name", "Dialogue"}, {"clips", QJsonArray{
                        clip("a1a", "wide", 0.0, 6.0), clip("a1b", "close", 6.0, 5.0, 2.0)}}},
                    QJsonObject{{"id", "A2"}, {"type", "audio"}, {"name", "Music"}, {"clips", QJsonArray{
                        clip("a2", "score", 0.0, 18.0)}}}}}};
        };
        const QJsonObject proposal{{"status", "pending"}, {"proposalId", "p7"},
                                   {"script", "trim v1b out -36f\nremove v1c\nadd title t2 at 00:00:12:00\n"},
                                   {"documentDiff", "-    clip v1b dur=5s\n+    clip v1b dur=3.5s\n-    clip v1c\n+    title t2\n"}};
        return QJsonObject{{"revision", revision}, {"duration", 18}, {"projectionStatus", projection},
                           {"timeline", timeline(false)}, {"pendingProposal", proposal}, {"proposalTimeline", timeline(true)}};
    }

    // Stand-in thumbnails and waveforms, so filmstrips and waves draw
    // without FFmpeg.
    static void fakeArtwork(MediaArtwork *artwork) {
        const auto frame = [](const QColor &sky, const QColor &ground) {
            QImage image(160, 90, QImage::Format_RGB32);
            QPainter painter(&image);
            QLinearGradient gradient(0, 0, 0, 90);
            gradient.setColorAt(0, sky.lighter(130));
            gradient.setColorAt(0.55, sky);
            gradient.setColorAt(0.56, ground);
            gradient.setColorAt(1, ground.darker(160));
            painter.fillRect(image.rect(), gradient);
            painter.setBrush(QColor(255, 240, 220));
            painter.setPen(Qt::NoPen);
            painter.drawEllipse(QPointF(118, 30), 9, 9);
            return image;
        };
        artwork->frames.insert(QStringLiteral("wide"), frame(QColor("#6f8fb5"), QColor("#2c4a5e")));
        artwork->frames.insert(QStringLiteral("close"), frame(QColor("#b58a6f"), QColor("#4e3b2c")));
        artwork->frames.insert(QStringLiteral("drone"), frame(QColor("#8fb5a8"), QColor("#2e5e4a")));
        for (const char *id : {"wide", "close", "score"}) {
            QImage wave(2048, 48, QImage::Format_ARGB32);
            wave.fill(Qt::transparent);
            QPainter painter(&wave);
            const double seed = qHash(QByteArray(id)) % 97;
            for (int x = 0; x < wave.width(); x += 2) {
                const double level = 0.25 + 0.7 * std::abs(std::sin(x * 0.013 + seed) * std::sin(x * 0.0021 + seed * 0.5));
                const int half = static_cast<int>(level * 23);
                painter.fillRect(QRect(x, 24 - half, 2, half * 2), Qt::white);
            }
            artwork->waves.insert(QString::fromLatin1(id), wave);
        }
        emit artwork->changed();
    }

    void editorRendersBothAppearances() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        // Back to the tests' dark appearance however this test ends.
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        QSignalSpy changed(EditorAppearance::instance(), &EditorAppearance::changed);
        const QJsonObject preset{{"name", "Harbor Intro"}, {"document", "kerf 1\nproject name=\"Harbor Intro\"\n"}, {"sequence", "s1"},
            {"slots", QJsonArray{
                QJsonObject{{"id", "Opening"}, {"seconds", 4}, {"video", true}, {"audio", true},
                    {"clips", QJsonArray{QJsonObject{{"id", "v1a"}, {"name", "Wide"}, {"sequence", "s1"}, {"track", "V1"}, {"at", "00:00:00:00"}}}},
                    {"fits", QJsonArray{QJsonObject{{"media", "wide"}, {"latestFrame", 380}}, QJsonObject{{"media", "close"}, {"latestFrame", 380}}}}},
                QJsonObject{{"id", "Logo"}, {"seconds", 2}, {"video", true}, {"audio", false},
                    {"clips", QJsonArray{QJsonObject{{"id", "t1"}, {"name", "Logo"}, {"sequence", "s1"}, {"track", "V2"}, {"at", "00:00:01:00"}}}},
                    {"fits", QJsonArray{QJsonObject{{"media", "drone"}, {"latestFrame", 432}}}}}}},
            {"textSlots", QJsonObject{{"title1", "Harbor Teaser"}}}};

        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            const bool dark = scheme == ColorScheme::Dark;
            const QString name = dark ? QStringLiteral("dark") : QStringLiteral("light");
            // Each picture starts from the Editing layout.
            QSettings().remove(QStringLiteral("workspace"));
            // The API KERF_COLOR_SCHEME feeds.
            setEditorColorScheme(scheme);
            QCOMPARE(editorColorScheme(), scheme);
            const EditorTheme &theme = editorTheme();
            QCOMPARE(theme.dark, dark);
            QVERIFY(dark ? theme.window.lightness() < 40 : theme.window.lightness() > 220);
            QVERIFY(dark ? theme.text.lightness() > 180 : theme.text.lightness() < 60);
            // Every token is filled in, and nothing amber or gold is left.
            const QString sheet = qApp->styleSheet() + dockStyleSheet(theme);
            QVERIFY(!sheet.contains(QLatin1Char('@')));
            static const QRegularExpression hex(QStringLiteral("#[0-9a-fA-F]{6,8}\\b"));
            for (auto it = hex.globalMatch(sheet); it.hasNext();) {
                const QString code = it.next().captured();
                QVERIFY2(!isAmber(QColor(code)), qPrintable(code));
            }
            // The system's Window and Accent stay the platform's, so
            // detection keeps reading the system and not the theme.
            QVERIFY(!QGuiApplication::palette().isBrushSet(QPalette::Active, QPalette::Accent));
            QVERIFY(!QGuiApplication::palette().isBrushSet(QPalette::Active, QPalette::Window));
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::Base), theme.field);
            // Clip labels read on their headers.
            for (const ClipKind kind : {ClipKind::Video, ClipKind::Audio, ClipKind::Music, ClipKind::Generator, ClipKind::Caption})
                QVERIFY(contrastRatio(theme.clip(kind).label, theme.clip(kind).header) >= 4.5);
            QVERIFY(contrastRatio(theme.accentText, theme.panel) >= 4.5);
            QVERIFY(contrastRatio(theme.onAssistant, theme.assistantStrong) >= 4.5);
            QCOMPARE(theme.viewerSurround, QColor("#1c1c1c"));

            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(showcaseSnapshot(12, QStringLiteral("clean")));
            window.selectClip(QStringLiteral("v1b"));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
            window.handleEvent(QJsonObject{{"type", "preset_library"}, {"directory", "presets"}, {"presets", QJsonArray{
                QJsonObject{{"path", "presets/harbor-intro.kerfpreset"}, {"name", "Harbor Intro"}, {"author", "jun"},
                            {"updated", "2026-10-30T09:00:00Z"}, {"slots", QJsonArray{
                                QJsonObject{{"id", "Opening"}, {"seconds", 4}, {"video", true}, {"audio", true}, {"clips", 2}},
                                QJsonObject{{"id", "Logo"}, {"seconds", 2}, {"video", true}, {"audio", false}, {"clips", 1}}}},
                            {"texts", QJsonObject{{"title1", "Title"}}}},
                QJsonObject{{"path", "presets/lower-thirds.kerfpreset"}, {"name", "Lower Thirds"}, {"author", "mina"},
                            {"updated", "2026-10-28T15:30:00Z"}, {"slots", QJsonArray{}}, {"texts", QJsonObject{{"name", "Name"}, {"role", "Role"}}}},
                QJsonObject{{"path", "presets/old.kerfpreset"}, {"name", "old"}, {"error", "line 3: unknown statement"}}}}});
            fakeArtwork(window.m_artwork);
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // Test builds have no runtime to send to; that is not this picture's subject.
            window.m_errorBanner->hide();
            // Nothing under the pointer.
            QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
            QVERIFY(window.m_timeline->hasProposal());
            QVERIFY(window.m_aiBar->isVisible());
            QCoreApplication::processEvents();
            const QImage image = window.grab().toImage();
            // The bars are the theme's: toolbar, status bar, and dock title strips.
            auto *toolbar = window.findChild<QToolBar *>();
            QVERIFY(toolbar);
            QCOMPARE(dominantColor(toolbar->grab().toImage()), theme.chrome.rgb());
            QCOMPARE(dominantColor(window.statusBar()->grab().toImage()), theme.statusBar.rgb());
            QVERIFY(dark ? qGray(dominantColor(toolbar->grab().toImage())) < 40 : qGray(dominantColor(toolbar->grab().toImage())) > 220);
            // The agent's bar is the assistant's purple, not amber.
            const QImage aiBar = window.m_aiBar->grab().toImage();
            QCOMPARE(dominantColor(aiBar), theme.assistantSurface.rgb());
            QCOMPARE(amberPixels(aiBar), 0);
            save(image, QStringLiteral("editor-%1").arg(name));

            // Settings.
            window.m_preferences->show();
            QVERIFY(QTest::qWaitForWindowExposed(window.m_preferences));
            QCoreApplication::processEvents();
            const QImage settings = window.m_preferences->grab().toImage();
            QCOMPARE(dominantColor(settings), theme.dialog.rgb());
            save(settings, QStringLiteral("settings-%1").arg(name));
            window.m_preferences->hide();

            // A representative dialog: the preset's media bindings.
            QTimer::singleShot(0, &window, [&] {
                auto *dialog = window.findChild<QDialog *>(QStringLiteral("presetBindingsDialog"));
                QVERIFY(dialog);
                QTimer::singleShot(2000, dialog, &QDialog::reject);
                if (auto *choice = dialog->findChild<QComboBox *>(QStringLiteral("presetMedia_Opening"))) choice->setCurrentIndex(1);
                QCoreApplication::processEvents();
                const QImage bindings = dialog->grab().toImage();
                QCOMPARE(dominantColor(bindings), theme.dialog.rgb());
                save(bindings, QStringLiteral("dialog-%1").arg(name));
                dialog->reject();
            });
            window.showPresetBindings(QStringLiteral("presets/harbor-intro.kerfpreset"), preset);

            // States: project.kerf changed on disk, the agent's proposal and
            // next steps, and a conversation in the assistant panel.
            window.applySnapshot(showcaseSnapshot(13, QStringLiteral("diverged")));
            showAssistant(window);
            window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
            window.appendConversation(QStringLiteral("Kerf"),
                QStringLiteral("I trimmed the close-up by 1.5 seconds, removed the drone pass, and added a lower third at 12 s."));
            window.appendConversation(QStringLiteral("Error"), QStringLiteral("The music cue could not be found on disk."));
            window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b · removed v1c"));
            window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                            QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music under the dialogue"}}});
            QCoreApplication::processEvents();
            window.m_errorBanner->hide();
            QCoreApplication::processEvents();
            QVERIFY(!window.m_divergedBanner->isHidden());
            QVERIFY(window.m_aiBar->isVisible());
            // The proposal's default button, Accept, is the accent.
            QPushButton *primary = window.m_acceptButton;
            QVERIFY(primary->isVisible() && primary->isEnabled());
            QCOMPARE(primary->objectName(), QStringLiteral("primaryButton"));
            const QImage primaryImage = primary->grab().toImage();
            const QColor fill = primaryImage.pixelColor(QPoint(6, primary->height() / 2) * primaryImage.devicePixelRatio());
            QVERIFY2(std::abs(fill.hue() - theme.accent.hue()) <= 4 && std::abs(fill.value() - theme.accent.value()) <= 24,
                     qPrintable(fill.name()));
            const QImage states = window.grab().toImage();
            QCOMPARE(amberPixels(window.m_divergedBanner->grab().toImage()), 0);
            save(states, QStringLiteral("states-%1").arg(name));
            window.close();
        }
        QVERIFY(changed.count() >= 2);

        // A change of appearance reaches a window that is already open: its
        // docks and the messages it has shown.
        setEditorColorScheme(ColorScheme::Light);
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("Done."));
        QCOMPARE(window.m_conversation->document()->rootFrame()->childFrames().constFirst()->frameFormat().background().color(),
                 editorTheme().messageAgent);
        const int before = changed.count();
        setEditorColorScheme(ColorScheme::Dark);
        QCOMPARE(changed.count(), before + 1);
        QVERIFY(window.m_workspace->manager()->styleSheet().contains(editorTheme().panelTitle.name()));
        QCOMPARE(window.m_conversation->document()->rootFrame()->childFrames().constFirst()->frameFormat().background().color(),
                 editorTheme().messageAgent);
        // Re-applying the same appearance changes nothing.
        applyEditorAppearance();
        QCOMPARE(changed.count(), before + 1);
    }

    // The viewers, the Source monitor's scrub bar, bin tiles and effect
    // cards draw in the theme in force, and the cached ones redraw when it
    // changes. The viewers keep the same dark field in both appearances.
    void paintedPanelsFollowTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        // A blue picture, on which outlines in the (blue) accent must still read.
        QImage picture(1920, 1080, QImage::Format_RGB32);
        {
            QPainter painter(&picture);
            QLinearGradient sky(0, 0, 0, 1080);
            sky.setColorAt(0, QColor("#9cc3ea"));
            sky.setColorAt(0.6, QColor("#3d78c2"));
            sky.setColorAt(1, QColor("#1b3350"));
            painter.fillRect(picture.rect(), sky);
        }
        // Made in the tests' dark appearance, so the loop below sees them redraw.
        MediaArtwork artwork;
        artwork.frames.insert(QStringLiteral("m1"), picture.scaled(160, 90));
        artwork.frames.insert(QStringLiteral("m2"), picture.scaled(160, 90));
        ProjectPanel bin(&artwork);
        bin.setSources(QJsonArray{
            QJsonObject{{"id", "m1"}, {"path", "media/a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m2"}, {"path", "media/b.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m3"}, {"path", "media/c.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m4"}, {"path", "media/d.wav"}, {"duration", 4}}});
        bin.setMediaStatus(QJsonObject{{"missing", QJsonArray{"m3"}}, {"pointers", QJsonArray{"m2"}}, {"external", QJsonArray{"m1"}}});
        bin.resize(560, 150);
        EffectsPanel effects;
        effects.setCatalog(QJsonArray{
            QJsonObject{{"kind", "effect"}, {"id", "light.edgeglow"}, {"name", "Edge Glow"}, {"category", "light"}},
            QJsonObject{{"kind", "effect"}, {"id", "blur.gaussian"}, {"name", "Gaussian Blur"}, {"category", "blur"}}});
        effects.setClip(QStringLiteral("c1"), true);
        QImage red(16, 9, QImage::Format_RGB32);
        red.fill(Qt::red);
        effects.setPreview(QStringLiteral("c1"), QStringLiteral("light.edgeglow"), red);
        effects.resize(420, 200);
        const auto tilePixel = [&bin](int row, const QPoint &at) {
            return bin.m_list->item(row)->icon().pixmap(QSize(112, 63)).toImage().pixelColor(at);
        };
        const auto cardPixel = [&effects](const char *id) {
            const QImage card = effects.m_items.value(QString::fromLatin1(id))->icon().pixmap(QSize(160, 90)).toImage();
            return card.pixelColor(card.width() / 2, 4);
        };

        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QString name = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");

            // Program monitor: a picture, a caption, and two masks, one selected.
            ViewerWidget viewer;
            viewer.resize(640, 380);
            viewer.setProjectInfo(1920, 1080);
            viewer.setFrame(picture, 0.0);
            viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
            viewer.setMasks(QStringLiteral("c1"), QJsonArray{
                QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.2, 0.15, 0.55, 0.6}}, {"mode", "add"}},
                QJsonObject{{"name", "k2"}, {"ellipse", QJsonArray{0.75, 0.4, 0.12, 0.2}}, {"mode", "none"}}},
                QTransform::fromScale(1920, 1080));
            viewer.selectMask(QStringLiteral("k1"));
            const QImage view = viewer.grab().toImage();
            const qreal ratio = view.devicePixelRatio();
            QCOMPARE(view.pixelColor(QPoint(2, 2) * ratio), theme.viewerSurround);
            const QPointF corner = viewer.pictureToWidget().map(QPointF(0.2, 0.15));
            QCOMPARE(view.pixelColor((corner * ratio).toPoint()), theme.maskHandleFill);
            QCOMPARE(amberPixels(view), 0);
            save(view, QStringLiteral("viewer-%1").arg(name));
            viewer.clearFrame();
            viewer.setMasks(QString(), {}, {});
            viewer.setCaption(QString());
            const QImage empty = viewer.grab().toImage();
            QCOMPARE(empty.pixelColor((viewer.videoRect().topLeft() + QPointF(6, 6)).toPoint() * ratio), theme.viewerCanvas);
            save(empty, QStringLiteral("viewer-empty-%1").arg(name));

            // The focus region picker.
            RegionPicker picker(picture.scaled(480, 270), QRectF(0.3, 0.2, 0.4, 0.5));
            picker.resize(520, 290);
            const QImage region = picker.grab().toImage();
            QCOMPARE(region.pixelColor(QPoint(2, 2) * region.devicePixelRatio()), theme.viewerSurround);
            QCOMPARE(amberPixels(region), 0);
            save(region, QStringLiteral("region-%1").arg(name));

            // Source monitor: the marked range and the position on the scrub bar.
            SourceMonitorPanel monitor;
            monitor.resize(480, 300);
            monitor.open(QJsonObject{{"id", "m4"}, {"path", "media/d.wav"}, {"duration", 4.0}, {"rate", "24"}}, QString());
            monitor.seek(1.0);
            monitor.markIn();
            monitor.seek(3.0);
            monitor.markOut();
            monitor.seek(2.0);
            monitor.show();
            QVERIFY(QTest::qWaitForWindowExposed(&monitor));
            QWidget *scrub = monitor.layout()->itemAt(1)->widget();
            QVERIFY(scrub && scrub->height() == 18);
            const QImage bar = scrub->grab().toImage();
            const qreal barRatio = bar.devicePixelRatio();
            const auto barX = [scrub](double seconds) { return static_cast<int>(8 + seconds / 4.0 * (scrub->width() - 16)); };
            QCOMPARE(bar.pixelColor(QPoint(barX(0.5), 9) * barRatio).rgb(), theme.scrubTrack.rgb());
            QCOMPARE(bar.pixelColor(QPoint(barX(1.5), 9) * barRatio).rgb(), theme.inOutRange.rgb());
            QCOMPARE(bar.pixelColor(QPoint(barX(2.0), 3) * barRatio).rgb(), theme.playhead.rgb());
            const QImage source = monitor.grab().toImage();
            QCOMPARE(source.pixelColor(QPoint(2, 2) * source.devicePixelRatio()), theme.viewerSurround);
            QCOMPARE(amberPixels(source), 0);
            save(source, QStringLiteral("source-%1").arg(name));
            monitor.close();

            // Bin tiles and their badges, redrawn for the new appearance.
            QCOMPARE(tilePixel(3, QPoint(2, 60)), theme.thumbnail);
            QCOMPARE(tilePixel(2, QPoint(5, 10)), theme.badgeError);
            QCOMPARE(tilePixel(1, QPoint(5, 10)).rgb(), theme.badgeInfo.rgb());
            bin.show();
            QVERIFY(QTest::qWaitForWindowExposed(&bin));
            const QImage tiles = bin.grab().toImage();
            QCOMPARE(amberPixels(tiles), 0);
            save(tiles, QStringLiteral("bin-%1").arg(name));

            // Effect cards: placeholders follow the theme, a preview stays.
            QCOMPARE(cardPixel("blur.gaussian"), theme.cardPlaceholder);
            QCOMPARE(cardPixel("light.edgeglow"), QColor(Qt::red));
            effects.show();
            QVERIFY(QTest::qWaitForWindowExposed(&effects));
            save(effects.grab().toImage(), QStringLiteral("effects-%1").arg(name));
        }
    }

    // Answer the next modal question with `button`, checking that it asks `text`.
    static void answerQuestion(QWidget *parent, QDialogButtonBox::StandardButton button, const QString &text) {
        QTimer::singleShot(0, parent, [button, text] {
            QWidget *modal = QApplication::activeModalWidget();
            auto *buttons = modal ? modal->findChild<QDialogButtonBox *>() : nullptr;
            if (!buttons || !buttons->button(button)) {
                if (modal) modal->close();
                QFAIL("no question");
            }
            const QString asked = modal->property("text").toString();
            buttons->button(button)->click();
            QCOMPARE(asked, text);
        });
    }

    void pullRequestsOpenWithoutGh() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject status = sharedStatus();
        status.insert("branch", "color");
        status.insert("gh", false);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));
        // Without gh the bridge answers with GitHub's page for starting one.
        const QString compare = QStringLiteral("https://github.com/jun/film/compare/color?expand=1");
        window.handleEvent(QJsonObject{{"type", "collab_pull_request"}, {"url", compare}, {"created", false}});
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Opened GitHub's page for a new pull request: ") + compare);
        const QString existing = QStringLiteral("https://github.com/jun/compare/pull/4");
        window.handleEvent(QJsonObject{{"type", "collab_pull_request"}, {"url", existing}, {"created", false}});
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("This branch already has a pull request: ") + existing);
    }

    void syncConflictsSayWhereAndWhoseVersionWasKept() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{}}, {"fastForward", false}, {"pushed", false},
            {"conflicts", QJsonArray{
                QJsonObject{{"path", "s1/V1/c3"}, {"what", "duration changed on both sides"}, {"kept", "ours"}},
                QJsonObject{{"path", "s1/V1/c9"}, {"what", "overlapped c5 on V1; moved to V2"}, {"kept", "theirs"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{}}}}});
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("Kept yours · s1/V1/c3: duration changed on both sides")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept theirs · s1/V1/c9: overlapped c5 on V1; moved to V2")));
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Synced: 1 change kept yours, 1 change kept theirs"));
    }

    void presetsReloadWhenTheWorkTreeChanges() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        // Switching branches brings in no commits, yet changes presets/.
        window.m_lastCommand = {};
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{}}, {"fastForward", false}, {"pushed", false}, {"conflicts", QJsonArray{}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{}}, {"branch", "color"}}}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));

        // So does a pull in a terminal, seen as project.kerf changing on disk,
        // and loading it.
        const QJsonObject timeline{{"fps", 24}, {"tracks", QJsonArray{}}};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 0}, {"projectionStatus", "clean"}, {"timeline", timeline}});
        window.m_lastCommand = {};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 0}, {"projectionStatus", "diverged"}, {"timeline", timeline}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
        window.m_loadFromDisk->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "projection_import"}}));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 0}, {"projectionStatus", "clean"}, {"timeline", timeline}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
        // An edit leaves the library alone.
        window.m_lastCommand = {};
        window.applySnapshot(QJsonObject{{"revision", 6}, {"duration", 0}, {"projectionStatus", "clean"}, {"timeline", timeline}});
        QCOMPARE(window.m_lastCommand, QJsonObject{});
    }

    void savingOverAPresetAsksAboutTheFileTheBridgeNamed() {
        QTemporaryDir project;
        MainWindow window{project.path(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject save{{"type", "preset_save"}, {"name", "Intro/Outro"}, {"replace", false}};
        // The bridge saves "Intro/Outro" as presets/Intro-Outro.kerfpreset.
        const QString exists = QStringLiteral("presets/Intro-Outro.kerfpreset already exists; choose another name or replace it");
        const QString question = QStringLiteral("presets/Intro-Outro.kerfpreset already exists. Replace it?");
        answerInput(&window, QStringLiteral("Intro/Outro"));
        window.m_presets->m_save->click();
        QCOMPARE(window.m_lastCommand, save);
        answerQuestion(&window, QDialogButtonBox::No, question);
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", exists}});
        QCOMPARE(window.m_lastCommand, save);

        answerInput(&window, QStringLiteral("Intro/Outro"));
        window.m_presets->m_save->click();
        answerQuestion(&window, QDialogButtonBox::Yes, question);
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", exists}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_save"}, {"name", "Intro/Outro"}, {"replace", true}}));

        // Once answered, the same refusal is only an error.
        QTimer::singleShot(1000, &window, [] {
            if (QWidget *modal = QApplication::activeModalWidget()) modal->close();
        });
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", exists}});
        QCOMPARE(window.m_errorMessage->text(), exists);
    }
};

} // namespace kerf

#include "editor_test.moc"

QTEST_MAIN(kerf::EditorTest)
