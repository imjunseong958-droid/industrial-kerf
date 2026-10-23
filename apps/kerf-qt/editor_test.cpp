#include "ask_prompt.h"
#include "command_registry.h"
#include "editor_common.h"
#include "effect_controls_panel.h"
#include "effects_panel.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
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
#include <QGroupBox>
#include <QCheckBox>
#include <QImage>
#include <QListWidget>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QMenuBar>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QScrollBar>
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
#include <QToolButton>
#include <QWidget>

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
        qApp->setStyleSheet(editorStyleSheet());
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
        QCOMPARE(menus, (QStringList{"File", "Edit", "Clip", "Sequence", "Markers", "Playback", "View", "Window"}));
        QVERIFY(window.findChild<QAction *>(QStringLiteral("keymap.premiere")));

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
        // The whole raster catalog is offered, by category and words.
        QVERIFY(panel->m_items.size() > 100);
        QVERIFY(panel->m_items.contains(QStringLiteral("stylize.glow")));
        QVERIFY(!panel->m_items.contains(QStringLiteral("transition.linearwipe")));
        panel->m_search->setText(QStringLiteral("neon"));
        QVERIFY(!panel->m_items.value(QStringLiteral("stylize.glow"))->isHidden());
        QVERIFY(panel->m_items.value(QStringLiteral("channel.invert"))->isHidden());
        window.selectClip(QStringLiteral("c1"));
        QCOMPARE(panel->m_clip, QStringLiteral("c1"));
        QVERIFY(panel->m_picture);
        // The cards shown ask for previews of themselves on the clip.
        panel->requestPreviews();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("c1"));
        QVERIFY(window.m_lastCommand.value("effects").toArray().contains(QJsonValue("stylize.glow")));
        QImage red(16, 9, QImage::Format_RGBA8888);
        red.fill(Qt::red);
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", "c1"}, {"fx", "stylize.glow"},
                                       {"width", 16}, {"height", 9},
                                       {"rgba", QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(red.constBits()), 16 * 9 * 4).toBase64())}});
        const QImage card = panel->m_items.value(QStringLiteral("stylize.glow"))->icon().pixmap(QSize(160, 90)).toImage();
        QCOMPARE(card.pixelColor(card.width() / 2, card.height() / 2), QColor(Qt::red));
        // Double-click adds the effect to the selected clip.
        emit panel->m_cards->itemActivated(panel->m_items.value(QStringLiteral("stylize.glow")));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(op.value("fx").toString(), QStringLiteral("stylize.glow"));
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
                                    QJsonObject{{"fx", "stylize.glow"}, {"enabled", true}, {"params", QJsonObject{{"radius", 40}}}},
                                    QJsonObject{{"fx", "channel.invert"}, {"enabled", false}, {"params", QJsonObject{}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        EffectControlsPanel *inspector = window.m_effectControls;
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 2);
        auto *glow = boxes.at(0)->findChild<QCheckBox *>(QStringLiteral("effectEnabled"));
        auto *radius = boxes.at(0)->findChild<QDoubleSpinBox *>(QStringLiteral("effectParam_radius"));
        QVERIFY(glow && radius);
        QCOMPARE(glow->text(), QStringLiteral("Glow"));
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
};

} // namespace kerf

#include "editor_test.moc"

QTEST_MAIN(kerf::EditorTest)
