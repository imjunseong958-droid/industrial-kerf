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

#include <QAbstractItemView>
#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDir>
#include <QDialogButtonBox>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGuiApplication>
#include <QGroupBox>
#include <QHBoxLayout>
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
#include <QPaintDevice>
#include <QPaintEngine>
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
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTextBlock>
#include <QTemporaryDir>
#include <QTemporaryDir>
#include <QTest>
#include <QTest>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QScreen>
#include <QWheelEvent>
#include <QWidget>

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

namespace {

// A paint device that keeps each run of text painted on it with the pen it
// was painted in, so a test reads a painted text's colour from the painting
// rather than from antialiased pixels. Everything else is dropped.
class TextRecorder final : public QPaintDevice {
public:
    struct Run {
        QString text;
        QColor color;
    };
    QList<Run> runs;

    TextRecorder() : m_engine(this) {}
    QPaintEngine *paintEngine() const override { return &m_engine; }
    // The colour the first run containing `text` was painted in.
    QColor colorOf(const QString &text) const {
        for (const Run &run : runs)
            if (run.text.contains(text)) return run.color;
        return {};
    }

protected:
    int metric(PaintDeviceMetric metric) const override {
        switch (metric) {
        case PdmWidth: return 600;
        case PdmHeight: return 120;
        case PdmWidthMM: return 212;
        case PdmHeightMM: return 42;
        case PdmNumColors: return INT_MAX;
        case PdmDepth: return 32;
        case PdmDpiX:
        case PdmDpiY:
        case PdmPhysicalDpiX:
        case PdmPhysicalDpiY: return 72;
        case PdmDevicePixelRatio: return 1;
        case PdmDevicePixelRatioScaled: return int(devicePixelRatioFScale());
        default: return QPaintDevice::metric(metric);
        }
    }

private:
    class Engine final : public QPaintEngine {
    public:
        explicit Engine(TextRecorder *device) : QPaintEngine(AllFeatures), m_device(device) {}
        bool begin(QPaintDevice *) override { return true; }
        bool end() override { return true; }
        void updateState(const QPaintEngineState &) override {}
        void drawPath(const QPainterPath &) override {}
        void drawPolygon(const QPointF *, int, PolygonDrawMode) override {}
        void drawPixmap(const QRectF &, const QPixmap &, const QRectF &) override {}
        void drawTextItem(const QPointF &, const QTextItem &item) override { m_device->runs.append({item.text(), state->pen().color()}); }
        Type type() const override { return User; }

    private:
        TextRecorder *m_device;
    };
    mutable Engine m_engine;
};

} // namespace

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
        useEditorStyle();
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
        // Every colour the welcome is written in reads at 4.5:1 on the
        // panel's well, in both appearances.
        {
            const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
            for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
                setEditorColorScheme(scheme);
                const QString html = window.m_conversation->toHtml();
                QVERIFY(html.contains(QStringLiteral("Try asking")));
                int colours = 0;
                for (const QRegularExpressionMatch &match : QRegularExpression(QStringLiteral("color:(#[0-9a-fA-F]{6})")).globalMatch(html)) {
                    const QColor ink(match.captured(1));
                    const qreal ratio = contrastRatio(ink, editorTheme().well);
                    QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1 on %2: %3").arg(ink.name(), editorTheme().well.name()).arg(ratio, 0, 'f', 2)));
                    ++colours;
                }
                QVERIFY(colours >= 4);
            }
        }
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
        // One view scrolls: the conversation is as tall as its messages.
        QCOMPARE(window.m_conversation->verticalScrollBar()->maximum(), 0);
        auto *scroll = window.m_chatScroll->verticalScrollBar();
        QVERIFY(scroll->maximum() > 0);
        // It followed the output to the latest.
        QCOMPARE(scroll->value(), scroll->maximum());
        scroll->setValue(0);
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("New response."));
        QCoreApplication::processEvents();
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
        // The composer's accent edge shows the focus, with no second ring
        // around the message field.
        QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("focusHalo")));
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
        {
            const QImage composer = window.m_chatComposer->grab().toImage();
            QCOMPARE(composer.pixelColor(QPoint(0, window.m_chatComposer->height() / 2) * composer.devicePixelRatio()).rgb(),
                     editorTheme().focusRing.rgb());
        }
        QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("focusHalo")));
        window.m_conversation->setFocus();
        QVERIFY(!window.m_chatComposer->isAncestorOf(QApplication::focusWidget()));
        QCOMPARE(window.m_chatComposer->property("state").toString(), QStringLiteral("ready"));
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
            // Kerf's conversation starts open, beside the Program monitor.
            QVERIFY(!workspace->panel(QStringLiteral("assistant"))->isClosed());
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

    // The Editing workspace reads as a video does on YouTube or TikTok on
    // the desktop: the Program monitor in the middle and the widest panel of
    // its row, Kerf's conversation in a column of its own on its right, the
    // full height of the row, the browsers and Effect Controls on its left,
    // the Source monitor a tab behind the Program monitor, and the Timeline
    // the window's full width under them all.
    void editingShowsTheVideoWithItsConversationBeside() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        const auto panel = [workspace](const char *id) { return workspace->panel(QString::fromLatin1(id)); };
        const auto area = [&panel](const char *id) { return panel(id)->dockAreaWidget(); };
        const auto frame = [&window](const QWidget *widget) { return QRect(widget->mapTo(&window, QPoint()), widget->size()); };

        QVERIFY(workspace->assistantOpen());
        QAction *toggle = window.m_commands->action(QStringLiteral("window.assistant"));
        QVERIFY(toggle->isChecked());
        QVERIFY(window.m_chatToggle && window.m_chatToggle->isChecked());
        QCOMPARE(window.m_chatToggle->defaultAction(), toggle);
        QCOMPARE(area("assistant")->openDockWidgetsCount(), 1);
        QCOMPARE(area("source"), area("program"));
        QVERIFY(panel("program")->isCurrentTab());
        for (const char *id : {"effectControls", "effects", "presets"}) QCOMPARE(area(id), area("project"));
        QVERIFY(panel("project")->isCurrentTab());
        // Their column shows the four tabs' names in full.
        for (const char *id : {"project", "effectControls", "effects", "presets"}) {
            auto *label = panel(id)->tabWidget()->findChild<ads::CElidingLabel *>();
            QVERIFY2(label && label->isVisible() && !label->isElided(), id);
        }
        // Every panel is open, and Window lists each one.
        for (const char *id : {"project", "source", "program", "timeline", "effectControls", "effects", "presets", "assistant"}) {
            QVERIFY2(!panel(id)->isClosed(), id);
            QAction *item = window.m_commands->action(QStringLiteral("window.%1").arg(QString::fromLatin1(id)));
            QVERIFY2(item && item->isCheckable() && item->isChecked(), id);
        }

        const QRect browser = frame(area("project"));
        const QRect viewer = frame(area("program"));
        const QRect chat = frame(area("assistant"));
        const QRect timeline = frame(area("timeline"));
        // One row: the browsers, the viewer, the conversation, side by side.
        QVERIFY(browser.right() < viewer.left() && viewer.right() < chat.left());
        QVERIFY(chat.left() - viewer.right() <= 3);
        QCOMPARE(chat.top(), viewer.top());
        QCOMPARE(chat.bottom(), viewer.bottom());
        QCOMPARE(browser.top(), viewer.top());
        QCOMPARE(chat.width(), Workspace::kAssistantWidth);
        QVERIFY(viewer.width() > browser.width() && viewer.width() > chat.width());
        // The Timeline spans the row under it, edge to edge.
        QCOMPARE(timeline.left(), browser.left());
        QCOMPARE(timeline.right(), chat.right());
        QVERIFY(timeline.top() > viewer.bottom());
        // The viewer has the larger share of the height.
        QVERIFY(viewer.height() > timeline.height());

        // A wider window widens the viewer; the columns beside it keep theirs.
        window.resize(1680, 900);
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("assistant")).width(), chat.width());
        QCOMPARE(frame(area("project")).width(), browser.width());
        QCOMPARE(frame(area("program")).width(), viewer.width() + 240);
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("program")).width(), viewer.width());

        // The toolbar's bubble closes the conversation and gives its width,
        // and the splitter's, to the viewer; the browsers keep theirs.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(panel("assistant")->isClosed());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked());
        QVERIFY(!area("assistant")->isVisible());
        QCOMPARE(frame(area("program")).left(), viewer.left());
        QCOMPARE(frame(area("program")).right(), chat.right());
        QCOMPARE(frame(area("project")).width(), browser.width());
        // And opens it again at its width.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(!panel("assistant")->isClosed() && toggle->isChecked());
        QCOMPARE(frame(area("assistant")), chat);
        QCOMPARE(frame(area("program")), viewer);

        // A width of one's own comes back too: closed from its header's
        // close button, opened from the keyboard.
        {
            ads::CDockSplitter *row = area("assistant")->parentSplitter();
            QList<int> sizes = row->sizes();
            const int column = row->indexOf(area("assistant"));
            sizes[column - 1] -= 60;
            sizes[column] += 60;
            row->setSizes(sizes);
            QCoreApplication::processEvents();
        }
        QCOMPARE(frame(area("assistant")).width(), chat.width() + 60);
        QToolButton *close = area("assistant")->findChild<QToolButton *>(QStringLiteral("assistantClose"));
        QVERIFY(close && close->isVisible());
        QVERIFY(close->toolTip().startsWith(QStringLiteral("Close Kerf Assistant")));
        // Its tab shows no second close button under the pointer.
        QWidget *tab = panel("assistant")->tabWidget();
        QEvent enter(QEvent::Enter);
        QCoreApplication::sendEvent(tab, &enter);
        QVERIFY(!tab->findChild<QWidget *>(QStringLiteral("tabCloseButton"))->isVisible());
        QTest::mouseClick(close, Qt::LeftButton);
        QCoreApplication::processEvents();
        QVERIFY(panel("assistant")->isClosed() && !toggle->isChecked());
        QCOMPARE(workspace->assistantWidth(), chat.width() + 60);
        QCOMPARE(toggle->shortcut(), QKeySequence(QStringLiteral("Ctrl+Shift+J")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(!panel("assistant")->isClosed() && window.m_chatToggle->isChecked());
        QCOMPARE(frame(area("assistant")).width(), chat.width() + 60);
        QCOMPARE(frame(area("assistant")).right(), chat.right());
        // Window ▸ Kerf Assistant is the same switch.
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(panel("assistant")->isClosed() && !window.m_chatToggle->isChecked());
        // Dragging the browsers' edge while it is closed (which leaves the
        // row nothing to remember for a hidden column) does not lose it.
        {
            ads::CDockSplitter *row = area("program")->parentSplitter();
            QWidget *handle = row->handle(row->indexOf(area("program")));
            QTest::mousePress(handle, Qt::LeftButton, {}, QPoint(1, 50));
            QTest::mouseMove(handle, QPoint(-30, 50));
            QTest::mouseRelease(handle, Qt::LeftButton, {}, QPoint(-30, 50));
            QCoreApplication::processEvents();
            QVERIFY(frame(area("project")).width() < browser.width());
            toggle->trigger();
            QCoreApplication::processEvents();
            QCOMPARE(frame(area("assistant")).width(), chat.width() + 60);
            QCOMPARE(frame(area("assistant")).right(), chat.right());
            toggle->trigger();
            QCoreApplication::processEvents();
        }
        // The AI bar's Details opens it, brought forward.
        QToolButton *details = nullptr;
        for (QToolButton *button : window.m_aiBar->findChildren<QToolButton *>())
            if (button->text() == QStringLiteral("Details")) details = button;
        QVERIFY(details);
        details->click();
        QCoreApplication::processEvents();
        QVERIFY(panel("assistant")->isCurrentTab() && toggle->isChecked() && window.m_chatToggle->isChecked());
        QCOMPARE(frame(area("assistant")).width(), chat.width() + 60);
        // The width is this Mac's, kept for the next launch, not the project's.
        window.close();
        QCOMPARE(QSettings().value(QStringLiteral("workspace/assistantWidth")).toInt(), chat.width() + 60);
    }

    // A conversation closed when the editor last closed opens, the next
    // time, at the width it had, however the layout restored it.
    void conversationOpensAtItsWidthAfterARelaunch() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        int width = 0;
        {
            MainWindow window{QString(), QString()};
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            ads::CDockAreaWidget *chat = window.m_workspace->panel(QStringLiteral("assistant"))->dockAreaWidget();
            ads::CDockSplitter *row = chat->parentSplitter();
            QList<int> sizes = row->sizes();
            const int column = row->indexOf(chat);
            sizes[column - 1] -= 40;
            sizes[column] += 40;
            row->setSizes(sizes);
            QCoreApplication::processEvents();
            width = chat->width();
            QCOMPARE(width, Workspace::kAssistantWidth + 40);
            window.m_workspace->setAssistantOpen(false);
            QCoreApplication::processEvents();
            // The browsers' edge moves while it is closed, so the saved
            // layout keeps nothing of the hidden column's width.
            ads::CDockAreaWidget *viewer = window.m_workspace->panel(QStringLiteral("program"))->dockAreaWidget();
            QWidget *handle = row->handle(row->indexOf(viewer));
            QTest::mousePress(handle, Qt::LeftButton, {}, QPoint(1, 50));
            QTest::mouseMove(handle, QPoint(-20, 50));
            QTest::mouseRelease(handle, Qt::LeftButton, {}, QPoint(-20, 50));
            QCoreApplication::processEvents();
            window.close();
        }
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QVERIFY(!window.m_workspace->assistantOpen());
        QVERIFY(!window.m_chatToggle->isChecked());
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        ads::CDockAreaWidget *chat = window.m_workspace->panel(QStringLiteral("assistant"))->dockAreaWidget();
        QCOMPARE(chat->width(), width);
        QCOMPARE(chat->mapTo(&window, QPoint(chat->width(), 0)).x(), window.width());
    }

    // The new Editing arrangement takes the place of the Editing layout
    // saved before it, while a workspace of one's own saved then keeps its
    // arrangement; one from before today's panels gives way to Editing's.
    void editingLayoutReplacesOldDefaultsAndKeepsOwnWorkspaces() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray before;
        QByteArray older;
        {
            MainWindow window{QString(), QString()};
            window.m_workspace->setAssistantOpen(false);
            window.m_workspace->panel(QStringLiteral("presets"))->toggleView(false);
            before = window.m_workspace->manager()->saveState(4);
            older = window.m_workspace->manager()->saveState(3);
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Review"), QStringLiteral("Old")});
            settings.setValue(QStringLiteral("workspace/saved/Editing"), before);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), before);
            settings.setValue(QStringLiteral("workspace/saved/Review"), before);
            settings.setValue(QStringLiteral("workspace/saved/Old"), older);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        ads::CDockWidget *presets = workspace->panel(QStringLiteral("presets"));
        QVERIFY(workspace->assistantOpen());
        QVERIFY(!presets->isClosed());
        workspace->activate(QStringLiteral("Review"));
        QVERIFY(!workspace->assistantOpen());
        QVERIFY(presets->isClosed());
        QVERIFY(!window.m_commands->action(QStringLiteral("window.assistant"))->isChecked());
        workspace->activate(QStringLiteral("Old"));
        QVERIFY(workspace->assistantOpen());
        QVERIFY(!presets->isClosed());
    }

    // In the conversation's narrow column Kerf's next steps wrap onto more
    // lines rather than widen it, and nothing in it scrolls sideways.
    void conversationFitsItsNarrowColumn() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
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
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
        QCoreApplication::processEvents();
        QWidget *column = window.findChild<QWidget *>(QStringLiteral("assistantPanel"));
        QVERIFY(column && column->isVisible());
        QCOMPARE(window.m_workspace->panel(QStringLiteral("assistant"))->dockAreaWidget()->width(), Workspace::kAssistantWidth);
        // The chips wrap: four of them take two lines, each inside the column.
        const auto chips = window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep"));
        QCOMPARE(chips.size(), 4);
        QSet<int> lines;
        for (QPushButton *chip : chips) {
            lines.insert(chip->y());
            QVERIFY(chip->geometry().right() < window.m_chatNextSteps->width());
        }
        QVERIFY(lines.size() >= 2);
        QVERIFY(window.m_chatNextSteps->height() >= 2 * chips.constFirst()->height());
        // Nothing asks for more width than the column has.
        QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
        QCOMPARE(window.m_chatScroll->widget()->width(), window.m_chatScroll->viewport()->width());
        QVERIFY(window.m_chatComposer->minimumSizeHint().width() <= window.m_chatComposer->width());
        QVERIFY(!window.m_proposalDiff->horizontalScrollBar()->isVisible());
        QVERIFY(!window.m_conversation->horizontalScrollBar()->isVisible());
        // The proposal's buttons and the composer's Send button sit inside it.
        for (QWidget *control : std::initializer_list<QWidget *>{window.m_acceptButton, window.m_rejectButton, window.m_sendButton})
            QVERIFY(control->mapTo(column, QPoint(control->width(), 0)).x() <= column->width());
        // Dragged narrower, the column stops where they still fit.
        ads::CDockAreaWidget *chat = window.m_workspace->panel(QStringLiteral("assistant"))->dockAreaWidget();
        ads::CDockSplitter *row = chat->parentSplitter();
        QList<int> sizes = row->sizes();
        const int index = row->indexOf(chat);
        sizes[index - 1] += sizes.at(index) - 80;
        sizes[index] = 80;
        row->setSizes(sizes);
        QCoreApplication::processEvents();
        QVERIFY(column->minimumWidth() >= 248);
        QCOMPARE(chat->width(), column->minimumWidth());
        QCOMPARE(column->width(), chat->width());
        QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
        for (QWidget *control : std::initializer_list<QWidget *>{window.m_acceptButton, window.m_sendButton})
            QVERIFY(control->mapTo(column, QPoint(control->width(), 0)).x() <= column->width());
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
        // The conversation, open beside the picture, shows them; the
        // timeline's bar shows them only while it is closed.
        QVERIFY(window.m_workspace->assistantOpen());
        QVERIFY(window.m_aiBar->isHidden());
        window.m_workspace->setAssistantOpen(false);
        QVERIFY(!window.m_aiBar->isHidden());
        QCOMPARE(window.m_aiBarText->text(), QStringLiteral("Next with Kerf"));
        QCOMPARE(window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep")).size(), 2);
        // Pressing one asks for it, about what the last request was about.
        chips.at(1)->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value("text").toString(), QStringLiteral("Use a linear wipe into the next shot."));
        QCOMPARE(window.m_lastCommand.value("focus").toObject(), focus);
    }

    // Kerf's conversation, in its column beside the picture: scrolled, the
    // top of its view fades from the well, so a message cut off there reads
    // as scrolled and not clipped; the proposal's script and diff are as
    // tall as their lines, up to ten, so what was asked stays in view.
    void conversationColumnFadesAndFitsItsProposal() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            QVERIFY(theme.messageUser.rgb() != theme.well.rgb());
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            QJsonObject proposal{{"status", "pending"}, {"proposalId", "p1"}, {"script", "trim v1b out -36f\nremove v1c\n"},
                                 {"documentDiff", "-    clip v1b dur=5s\n+    clip v1b dur=3.5s\n-    clip v1c\n"}};
            QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
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

            // The diff: no room left under its last line, and no scroll bar.
            QTextEdit *diff = window.m_proposalDiff;
            QVERIFY(diff->isVisible());
            const int line = diff->fontMetrics().lineSpacing();
            const qreal content = diff->document()->size().height();
            QVERIFY2(diff->viewport()->height() - content < line,
                     qPrintable(QStringLiteral("%1 px for %2 px of lines").arg(diff->viewport()->height()).arg(content)));
            QVERIFY(diff->viewport()->height() >= content - 1);
            QCOMPARE(diff->verticalScrollBar()->maximum(), 0);
            // A long one stops at ten lines and scrolls.
            QString longScript;
            for (int index = 0; index < 40; ++index) longScript += QStringLiteral("trim v%1 out -12f\n").arg(index);
            proposal.insert(QStringLiteral("script"), longScript);
            snapshot.insert(QStringLiteral("pendingProposal"), proposal);
            snapshot.insert(QStringLiteral("revision"), 4);
            window.applySnapshot(snapshot);
            QCoreApplication::processEvents();
            QVERIFY(diff->viewport()->height() <= MainWindow::kProposalDiffLines * line + 2 * diff->document()->documentMargin() + 1);
            QVERIFY(diff->viewport()->height() >= (MainWindow::kProposalDiffLines - 1) * line);
            QVERIFY(diff->verticalScrollBar()->maximum() > 0);

            // Scrolled to the top, no fade.
            QScrollArea *scroll = window.m_chatScroll;
            auto *fade = scroll->findChild<QWidget *>(QStringLiteral("scrollFade"), Qt::FindDirectChildrenOnly);
            QVERIFY(fade);
            QVERIFY(scroll->verticalScrollBar()->maximum() > 0);
            scroll->verticalScrollBar()->setValue(0);
            QCoreApplication::processEvents();
            QVERIFY(!fade->isVisible());
            // Scrolled so a message of the reader's is cut by the view's top.
            QTextDocument *document = window.m_conversation->document();
            QTextFrame *message = nullptr;
            for (QTextFrame *frame : document->rootFrame()->childFrames())
                if (frame->frameFormat().background().color() == theme.messageUser) message = frame;
            QVERIFY(message);
            const QRectF bounds = document->documentLayout()->frameBoundingRect(message);
            const int into = window.m_conversation->viewport()->mapTo(scroll->widget(), QPoint(0, int(bounds.top()) + 10)).y();
            scroll->verticalScrollBar()->setValue(into);
            QCoreApplication::processEvents();
            QCOMPARE(scroll->verticalScrollBar()->value(), into);
            QVERIFY(fade->isVisible());
            QCOMPARE(fade->geometry().top(), scroll->viewport()->geometry().top());
            QCOMPARE(fade->width(), scroll->viewport()->width());
            const QImage image = window.grab().toImage();
            const qreal ratio = image.devicePixelRatio();
            // Inside the message's padding, clear of its text and border.
            const QPoint column = window.m_conversation->viewport()->mapTo(&window, QPoint(int(bounds.right()) - 8, 0));
            const QPoint top(column.x(), fade->mapTo(&window, QPoint()).y());
            QCOMPARE(image.pixelColor(top * ratio).rgb(), theme.well.rgb());
            // Under the fade, the message as it is.
            QCOMPARE(image.pixelColor((top + QPoint(0, MainWindow::kScrollFadeHeight + 1)) * ratio).rgb(), theme.messageUser.rgb());
        }
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
        // A second mask is set apart as a second effect is: a hairline over
        // it and the same 18px lead. The section's title and the masks'
        // names are semibold, as the effects' names are.
        {
            QJsonObject k2 = k1;
            k2.insert("name", "k2");
            snapshot(9, QJsonArray{k1, k2});
            const auto pair = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
            QCOMPARE(pair.size(), 2);
            inspector->ensurePolished();
            QVERIFY(!pair.at(0)->property("following").toBool());
            QVERIFY(pair.at(1)->property("following").toBool());
            QCOMPARE(inspector->m_masksLayout->spacing() + pair.at(1)->layout()->contentsMargins().top(), 18);
            const QImage second = pair.at(1)->grab().toImage();
            QCOMPARE(second.pixelColor(second.width() / 2, 0), editorTheme().divider);
            QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("inspectorSection"))->font().weight(), QFont::DemiBold);
            for (const QWidget *box : pair)
                QCOMPARE(box->findChild<QToolButton *>(QStringLiteral("maskName"))->font().weight(), QFont::DemiBold);
        }
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
        QVERIFY(!window.m_effectControls->m_propertyForm->parentWidget()->isHidden());
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
        // A colour well: the swatch, with the value behind it rather than beside it.
        QCOMPARE(color->property("modelColor").toString(), QStringLiteral("#ffffff"));
        QVERIFY(color->text().isEmpty());
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
        // A file parameter is a field with a Choose… button 8px after it, as
        // the launcher's Location row, not a full-colour folder in the field.
        QWidget *file = window.m_effectControls->valueEditor(
            QJsonObject{{"name", "lut"}, {"type", "file"}, {"extensions", QJsonArray{"cube"}}, {"default", ""}},
            QStringLiteral("LUT"), [](const QJsonValue &) {});
        auto *fileLine = file->findChild<QLineEdit *>();
        auto *fileChoose = file->findChild<QPushButton *>();
        QVERIFY(fileLine && fileChoose);
        QVERIFY(fileLine->actions().isEmpty());
        QCOMPARE(fileChoose->text(), QStringLiteral("Choose…"));
        QVERIFY(fileChoose->icon().isNull());
        file->resize(240, 22);
        file->layout()->activate();
        QCOMPARE(fileChoose->x() - fileLine->geometry().right() - 1, 8);
        QCOMPARE(fileChoose->geometry().right(), file->width() - 1);
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
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 36, kTrackHeight / 2));
        QCOMPARE(seeks.constLast()[0].toDouble(), 0.5);
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 144, y));
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 216, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 216, y));
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("clip"));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        for (const bool leading : {true, false}) {
            const int edge = kTimelineOrigin + (leading ? 75 : 357);
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
            return kTimelineOrigin + int(std::lround(seconds * (timeline.width() - kTimelineOrigin) / 8.0));
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
        window.m_workspace->setAssistantOpen(false);
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
        // Reviewed in the conversation while it is open, on the timeline's
        // bar while it is closed.
        QVERIFY(!window.m_aiBar->isVisible());
        window.m_workspace->setAssistantOpen(false);
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

    // The timeline paints from the theme, in light and in dark, and repaints
    // its cached lanes when the appearance changes under it.
    void timelineFollowsTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        setEditorColorScheme(ColorScheme::Light);
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        const QJsonObject snapshot = showcaseSnapshot(1, QStringLiteral("clean"));
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(snapshot.value("timeline").toObject(), 18, 24);
        timeline.setProposal(snapshot.value("proposalTimeline").toObject());
        timeline.setSelectedClip(QStringLiteral("v1b"));
        timeline.setPlayhead(7.5);
        timeline.resize(timeline.minimumWidth(), 360);
        timeline.show();
        const auto x = [](double seconds) { return kTimelineOrigin + int(std::lround(seconds * 72.0)); };
        // Rows, top down: V3 captions, V2 titles, V1, A1 dialogue, A2 music.
        const int captions = kRulerHeight + kVideoTrackHeight / 2;
        const int titles = captions + kVideoTrackHeight;
        const int music = kRulerHeight + 3 * kVideoTrackHeight + kTrackHeight + kTrackHeight / 2;
        const QRect selected = timeline.clipBounds(QStringLiteral("v1b"));
        const QRect unselected = timeline.clipBounds(QStringLiteral("v1a"));
        // Dragging v1b by this much lands its end near the caption's end at 13 s.
        const QPoint grip = selected.center();
        const QPoint drop = grip + QPoint(x(13.0) - x(11.0) - 5, 0);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            // Switched under the open timeline, with nothing else to invalidate its lanes.
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("timeline-%1.png").arg(theme.dark ? "dark" : "light"))));
            QCOMPARE(image.pixelColor(4, 4), theme.rulerCorner);
            QCOMPARE(image.pixelColor(x(9.0), titles), theme.trackLane);
            QCOMPARE(image.pixelColor(x(7.5), titles), theme.playhead);
            QCOMPARE(image.pixelColor(selected.right(), selected.center().y()), theme.clipSelectedRingOuter);
            QCOMPARE(image.pixelColor(selected.right() - 1, selected.center().y()), theme.clipSelectedRing);
            QCOMPARE(image.pixelColor(unselected.left(), unselected.center().y()), theme.clipVideo.border);
            // The clip the proposal removes is outlined in red to its corners.
            const QRect removed = timeline.clipBounds(QStringLiteral("v1c"));
            for (const QPoint &corner : {removed.topLeft(), removed.topRight(), removed.bottomLeft(), removed.bottomRight()})
                QCOMPARE(image.pixelColor(corner), theme.proposalRemove);
            // A header's stripe is its clips' colour.
            QCOMPARE(image.pixelColor(1, captions), theme.clipCaption.header);
            QCOMPARE(image.pixelColor(1, titles), theme.clipGenerator.header);
            QCOMPARE(image.pixelColor(1, music), theme.clipMusic.header);
            QCOMPARE(amberPixels(image), 0);

            // A drag shows the edit point it snaps to, until it drops.
            QCOMPARE(image.pixelColor(x(13.0), captions), theme.trackLane);
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, grip);
            QTest::mouseMove(&timeline, drop);
            const QImage dragging = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(dragging.save(QDir(shots).filePath(QStringLiteral("timeline-drag-%1.png").arg(theme.dark ? "dark" : "light"))));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, drop);
            QVERIFY(std::abs(qGray(dragging.pixel(x(13.0), captions)) - qGray(theme.trackLane.rgb())) > 100);
            QCOMPARE(amberPixels(dragging), 0);
            // Where it will overwrite v1c (11 s to its new end at 13 s), a
            // dashed edge inside its selection ring, which stays whole:
            // white dashes on a dark line, in either appearance.
            {
                const int top = selected.top();
                const int bottom = selected.bottom();
                int dashes = 0;
                for (int px = x(11.0) + 4; px <= x(13.0) - 5; ++px) {
                    QCOMPARE(dragging.pixelColor(px, top), theme.clipSelectedRingOuter);
                    QCOMPARE(dragging.pixelColor(px, top + 1), theme.clipSelectedRing);
                    QCOMPARE(dragging.pixelColor(px, bottom - 1), theme.clipSelectedRing);
                    QCOMPARE(dragging.pixelColor(px, bottom), theme.clipSelectedRingOuter);
                    dashes += dragging.pixel(px, top + 3) == qRgb(255, 255, 255) ? 1 : 0;
                }
                QVERIFY(dashes > 8);
            }
            QCOMPARE(timeline.grab().toImage().pixelColor(x(13.0), captions), theme.trackLane);
        }
        // At 00:00:00, where every project opens, the playhead's head shows
        // whole, clear of the track headers.
        timeline.setPlayhead(0.0);
        const QImage start = timeline.grab().toImage();
        for (int y = 6; y <= 14; ++y)
            for (int px = kTimelineOrigin - 4; px <= kTimelineOrigin + 4; ++px)
                QCOMPARE(start.pixelColor(px, y), editorTheme().playhead);
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
        QCOMPARE(timeline.addLaneButtonCenter().y(), kRulerHeight + kTrackHeight + (kAddLaneHeight - 1) / 2);
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
        // A preview needs a header of two lines.
        timeline.setAllTrackHeights(kDetailedTrackHeight);
        const QPoint thumbnail(kTimelineLabelWidth - 9 - kHeaderPreviewWidth / 2, kRulerHeight + kDetailedTrackHeight / 2);
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), editorTheme().trackHeader);
        QImage frame(160, 90, QImage::Format_RGB32);
        frame.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("sample"), frame);
        emit artwork.changed();
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#e87342"));
        // A one-line header, at the default height or any below two lines,
        // has no room for it.
        for (const int height : {kDetailedTrackHeight - 1, kVideoTrackHeight, kMinTrackHeight}) {
            timeline.setAllTrackHeights(height);
            QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail.x(), kRulerHeight + height / 2), editorTheme().trackHeader);
        }
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
                          QPoint(kTimelineOrigin + 100, timeline.trackHeaderRect(0).center().y()));
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
        for (int i = 0; i < 12; ++i)
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
        QCOMPARE(view.pixelColor(4, 4), editorTheme().rulerCorner);
        QCOMPARE(view.pixelColor(6, 60), editorTheme().trackHeader);
        QCOMPARE(view.pixelColor(kTimelineLabelWidth + 50, kRulerHeight - 1), editorTheme().divider);

        QSignalSpy seeks(timeline, &TimelineWidget::seekRequested);
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QCOMPARE(seeks.count(), 0);
        QTest::mouseMove(timeline, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QVERIFY(timeline->skimLabel().isEmpty());
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier,
                          timeline->mapFrom(scroll.viewport(), QPoint(kTimelineOrigin + 72, 10)));
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

    // Every track's height, top to bottom.
    static QList<int> trackHeights(const TimelineWidget &timeline) {
        QList<int> heights;
        for (int row = 0; row < timeline.trackRowCount(); ++row) heights << timeline.trackHeaderRect(row).height();
        return heights;
    }
    static QJsonObject trackList(const QStringList &kinds) {
        QJsonArray tracks;
        for (int index = 0; index < kinds.size(); ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("T%1").arg(index)}, {"type", kinds.at(index)}, {"clips", QJsonArray{}}});
        return QJsonObject{{"fps", 24}, {"tracks", tracks}};
    }

    // The skimmed playhead keeps the playhead's halo, so its line reads at
    // 3:1 where it crosses clips and pictures, not only on bare lanes.
    void skimmedPlayheadKeepsItsHaloOverClips() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.resize(800, 200);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c"}, {"kind", "media"}, {"timelineStart", 0.0}, {"duration", 6.0}}}}}}}}, 8, 24);
        timeline.setPlayhead(2.0);
        timeline.show();
        const int playX = kTimelineOrigin + 144;
        const QRect clip = timeline.clipBounds(QStringLiteral("c"));
        QVERIFY(clip.left() < playX - 1 && clip.right() > playX + 1);
        // In the clip's body, under its name bar.
        const int y = clip.bottom() - 4;
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + (scheme == ColorScheme::Dark ? 420 : 400), clip.center().y()));
            QVERIFY(!timeline.skimLabel().isEmpty());
            const QImage skimmed = timeline.grab().toImage();
            QCOMPARE(skimmed.pixel(playX, y), theme.playheadIdle.rgb());
            QCOMPARE(skimmed.pixel(playX - 1, y), theme.playheadHalo.rgb());
            QCOMPARE(skimmed.pixel(playX + 1, y), theme.playheadHalo.rgb());
            QVERIFY(contrastRatio(theme.playheadIdle, theme.playheadHalo) >= 3.0);
            // Not skimming, the playhead has the same halo.
            timeline.clearSkimmer();
            const QImage playing = timeline.grab().toImage();
            QCOMPARE(playing.pixel(playX, y), theme.playhead.rgb());
            QCOMPARE(playing.pixel(playX - 1, y), theme.playheadHalo.rgb());
            QCOMPARE(playing.pixel(playX + 1, y), theme.playheadHalo.rgb());
        }
    }

    // The header bar sets every track's height at once (Mini, the default
    // Medium, Large, and Fit) and shows which is in force; in the editor it
    // is in the Timeline panel's title strip, before the track count.
    void timelineHeaderBarSetsEveryTrackHeight() {
        TimelineWidget timeline;
        timeline.resize(900, 520);
        timeline.setProject(trackList({"video", "video", "audio", "audio"}), 8, 24);
        timeline.show();
        const QList<QAction *> actions = timeline.trackHeightActions();
        QStringList names;
        for (const QAction *action : actions) names << action->objectName();
        QCOMPARE(names, (QStringList{"trackHeightMini", "trackHeightMedium", "trackHeightLarge", "trackHeightFit"}));
        QAction *mini = actions.at(0);
        QAction *medium = actions.at(1);
        QAction *large = actions.at(2);
        QAction *fit = actions.at(3);
        for (QAction *action : actions) QVERIFY(action->toolTip().contains(QStringLiteral("Option-scroll")));
        // Tracks start dense: one-line headers, a filmstrip on a picture
        // track (a 14px body) and a waveform on a sound track (a 10px one);
        // Mini is one line of 18px.
        QVERIFY(kVideoTrackHeight < kDetailedTrackHeight && kVideoTrackHeight >= 34);
        QVERIFY(kTrackHeight <= kVideoTrackHeight && kTrackHeight >= 30);
        QCOMPARE(kMinTrackHeight, 18);
        QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kVideoTrackHeight, kTrackHeight, kTrackHeight}));
        QVERIFY(medium->isChecked() && !mini->isChecked() && !large->isChecked() && !fit->isCheckable());
        mini->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(4, kMinTrackHeight));
        QVERIFY(mini->isChecked() && !medium->isChecked());
        large->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(4, 60));
        QVERIFY(large->isChecked() && !mini->isChecked());
        // One track dragged apart: no height is every track's.
        const int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge));
        QTest::mouseMove(&timeline, QPoint(140, edge + 10));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 10));
        QCOMPARE(trackHeights(timeline), (QList<int>{70, 60, 60, 60}));
        QVERIFY(!mini->isChecked() && !medium->isChecked() && !large->isChecked());
        medium->trigger();
        QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kVideoTrackHeight, kTrackHeight, kTrackHeight}));
        QVERIFY(medium->isChecked());
        fit->trigger();
        const int each = (timeline.height() - kRulerHeight - kAddLaneHeight - 8) / 4;
        QCOMPARE(trackHeights(timeline), QList<int>(4, std::clamp(each, kMinTrackHeight, kMaxTrackHeight)));
        // Their symbols are the glyph colour, and the accent when chosen.
        QCOMPARE(ink(mini->icon()), editorTheme().glyph.rgb());
        QCOMPARE(ink(mini->icon(), QIcon::Normal, QIcon::On), editorTheme().accentText.rgb());

        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject project = trackList({"video", "video", "audio"});
        project.insert("width", 1920);
        project.insert("height", 1080);
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", project}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ads::CDockWidget *dock = window.m_workspace->panel(QStringLiteral("timeline"));
        QCOMPARE(dock->titleBarActions().mid(0, 4), window.m_timeline->trackHeightActions());
        QCOMPARE(dock->titleBarActions().constLast()->objectName(), QStringLiteral("timelineDetail"));
        auto *button = dock->dockAreaWidget()->findChild<QToolButton *>(QStringLiteral("trackHeightMini"));
        QVERIFY(button && button->isVisible());
        QTest::mouseClick(button, Qt::LeftButton);
        QCOMPARE(trackHeights(*window.m_timeline), QList<int>(3, kMinTrackHeight));
        QVERIFY(button->isChecked());
        // Chosen, it looks as a chosen mask tool does: the accent's wash.
        const QImage chosen = button->grab().toImage();
        QCOMPARE(chosen.pixelColor(QPoint(3, button->height() / 2) * chosen.devicePixelRatio()).rgb(), editorTheme().accentTint.rgb());
    }

    // Option-Up and Option-Down, and Option-scroll over the headers, step
    // each track through the heights from its own, keeping in place the
    // track at the top of the view or under the pointer; plain keys and the
    // wheel elsewhere leave heights alone.
    void timelineStepsEveryTrackWithOptionKeysAndScroll() {
        TimelineScrollArea scroll;
        scroll.resize(900, 400);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        QStringList kinds;
        for (int index = 0; index < 40; ++index) kinds << (index % 4 == 3 ? "audio" : "video");
        timeline->setProject(trackList(kinds), 8, 24);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Rows run V30 … V1, then A1 … A10.
        const auto heights = [](int picture, int sound) {
            QList<int> expected(30, picture);
            expected.append(QList<int>(10, sound));
            return expected;
        };
        QCOMPARE(trackHeights(*timeline), heights(kVideoTrackHeight, kTrackHeight));
        timeline->setFocus();
        QTest::keyClick(timeline, Qt::Key_Up, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(44, 36));
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(30, 26));
        QTest::keyClick(timeline, Qt::Key_Down);
        QCOMPARE(trackHeights(*timeline), heights(30, 26));

        const auto wheel = [timeline](const QPoint &at, int delta, Qt::KeyboardModifiers modifiers,
                                      Qt::ScrollPhase phase = Qt::NoScrollPhase) {
            QWheelEvent event(at, timeline->mapToGlobal(at), QPoint(), QPoint(0, delta), Qt::NoButton, modifiers, phase, false);
            QApplication::sendEvent(timeline, &event);
            return event.isAccepted();
        };
        const auto inView = [timeline, &scroll](const QPoint &point) { return timeline->mapFrom(scroll.viewport(), point); };
        QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(36, 30));
        // A trackpad's small deltas add up to a notch.
        for (int part = 0; part < 3; ++part) wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(30, 26));
        // Without Option, or over the lanes, the wheel is the scroll area's.
        QVERIFY(!wheel(inView(QPoint(120, 200)), 120, Qt::NoModifier));
        QVERIFY(!wheel(inView(QPoint(500, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(30, 26));
        // The momentum after a flick steps nothing, and a new gesture starts
        // its notch afresh.
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollBegin));
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollUpdate));
        QCOMPARE(trackHeights(*timeline), heights(36, 30));
        for (int event = 0; event < 6; ++event) QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier, Qt::ScrollMomentum));
        QCOMPARE(trackHeights(*timeline), heights(36, 30));
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollEnd);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollBegin);
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(36, 30));
        wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(30, 26));

        // The track under the pointer stays where it is in the view.
        scroll.verticalScrollBar()->setValue(300);
        QCoreApplication::processEvents();
        const QPoint over = inView(QPoint(120, 250));
        const int row = timeline->trackRowAt(over.y());
        QVERIFY(row > 5);
        const auto top = [timeline](int at) { return timeline->trackHeaderRect(at).top() + timeline->pos().y(); };
        const int before = top(row);
        wheel(over, -120, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        QCOMPARE(top(row), before);
        wheel(inView(QPoint(120, 250)), 240, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(36, 30));
        QCOMPARE(top(row), before);
        // The keys keep the track at the top of the view in place.
        const int first = timeline->trackRowAt(-timeline->pos().y() + kRulerHeight);
        const int firstTop = top(first);
        QTest::keyClick(timeline, Qt::Key_Up, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(44, 36));
        QCOMPARE(top(first), firstTop);
    }

    // A step taller never makes a track shorter: picture tracks dragged tall
    // and one hero layer taller still each step from their own heights, the
    // sound tracks from theirs, and a track added later from the height
    // every track was stepped to.
    void timelineStepsKeepTracksSizedApart() {
        TimelineWidget timeline;
        timeline.resize(900, 900);
        QJsonObject project = trackList({"video", "video", "video", "audio", "audio"});
        timeline.setProject(project, 8, 24);
        timeline.show();
        // Rows, top down: V3, V2, V1, A1, A2. Option-drag picture to 100.
        const int pictureEdge = timeline.trackHeaderRect(2).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(140, pictureEdge + 100 - kVideoTrackHeight));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge + 100 - kVideoTrackHeight));
        QCOMPARE(trackHeights(timeline), (QList<int>{100, 100, 100, kTrackHeight, kTrackHeight}));
        // V3, the hero layer, taller still.
        const int heroEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, heroEdge));
        QTest::mouseMove(&timeline, QPoint(140, heroEdge + 20));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, heroEdge + 20));
        QCOMPARE(trackHeights(timeline), (QList<int>{120, 100, 100, kTrackHeight, kTrackHeight}));
        timeline.stepAllTrackHeights(1);
        const QList<int> taller = trackHeights(timeline);
        QCOMPARE(taller, (QList<int>{128, 104, 104, 36, 36}));
        timeline.stepAllTrackHeights(-1);
        QCOMPARE(trackHeights(timeline), (QList<int>{104, 86, 86, 30, 30}));
        // The tallest stays the tallest; Option-scroll steps the same way.
        QWheelEvent wheel(QPointF(120, timeline.trackHeaderRect(3).center().y()), timeline.mapToGlobal(QPointF(120, 0)), QPoint(),
                          QPoint(0, 120), Qt::NoButton, Qt::AltModifier, Qt::NoScrollPhase, false);
        QApplication::sendEvent(&timeline, &wheel);
        QCOMPARE(trackHeights(timeline), (QList<int>{128, 104, 104, 36, 36}));
        // A new track takes the step every track without a height of its
        // own was taken to: one step from the picture default.
        QJsonArray tracks = project.value("tracks").toArray();
        tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
        project.insert("tracks", tracks);
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.allTrackHeight(), 44);
        QCOMPARE(trackHeights(timeline).constLast(), 44);
        // Presets still set every track alike.
        timeline.trackHeightActions().at(0)->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(6, kMinTrackHeight));
    }

    // At every height down to Mini the line between two headers shows the
    // resize cursor and says what dragging it does; dragging it resizes the
    // track above, Option-dragging every track of that kind; and a header's
    // toggles keep their clicks, however close to the line.
    void timelineHeaderEdgesResizeAtEveryHeight() {
        TimelineWidget timeline;
        timeline.resize(800, 400);
        timeline.setProject(trackList({"video", "video", "video", "audio", "audio"}), 8, 24);
        timeline.setAllTrackHeights(kMinTrackHeight);
        timeline.show();
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        // Rows, top down: V3, V2, V1, A1, A2. This line parts V2 from V1.
        const int edge = timeline.trackHeaderRect(1).bottom() + 1;
        for (const int dy : {-2, 0, 2}) {
            QTest::mouseMove(&timeline, QPoint(140, edge + dy));
            QCOMPARE(timeline.cursor().shape(), Qt::SplitVCursor);
        }
        QHelpEvent help(QEvent::ToolTip, QPoint(140, edge), timeline.mapToGlobal(QPoint(140, edge)));
        QApplication::sendEvent(&timeline, &help);
        QVERIFY2(QToolTip::text().contains(QStringLiteral("Option-drag")), qPrintable(QToolTip::text()));
        QToolTip::hideText();

        const QRect lock = timeline.headerToggleRect(2, 1);
        QVERIFY(lock.top() > edge && lock.top() - edge <= 3);
        QTest::mouseMove(&timeline, lock.center());
        QVERIFY(timeline.cursor().shape() != Qt::SplitVCursor);
        for (const QPoint &at : {lock.center(), QPoint(lock.center().x(), lock.top())}) {
            QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, at);
            QCOMPARE(toggles.constLast().at(1).toString(), QStringLiteral("locked"));
        }
        QCOMPARE(toggles.count(), 2);
        QCOMPARE(trackHeights(timeline), QList<int>(5, kMinTrackHeight));

        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge - 1));
        QTest::mouseMove(&timeline, QPoint(140, edge + 19));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 19));
        QCOMPARE(trackHeights(timeline), (QList<int>{18, 38, 18, 18, 18}));
        // Option: every sound track...
        const int soundEdge = timeline.trackHeaderRect(3).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, soundEdge));
        QTest::mouseMove(&timeline, QPoint(140, soundEdge + 12));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, soundEdge + 12));
        QCOMPARE(trackHeights(timeline), (QList<int>{18, 38, 18, 30, 30}));
        // ...or every picture track.
        const int pictureEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(140, pictureEdge + 6));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge + 6));
        QCOMPARE(trackHeights(timeline), (QList<int>{24, 24, 24, 30, 30}));
        QCOMPARE(toggles.count(), 2);
    }

    // Track heights are remembered per project in the editor's settings
    // (never in project.kerf, which collaborators share), restored when the
    // project opens again; a track added later takes the height every
    // track was last set to.
    void timelineRemembersTrackHeightsPerProject() {
        QSettings().remove(QStringLiteral("timelineTrackHeights"));
        const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("timelineTrackHeights")); });
        QTemporaryDir first;
        QTemporaryDir second;
        const QJsonObject project = trackList({"video", "audio"});
        {
            TimelineWidget timeline;
            timeline.resize(700, 400);
            timeline.setHeightsKey(first.path());
            timeline.setProject(project, 8, 24);
            timeline.show();
            timeline.setAllTrackHeights(26);
            const int edge = timeline.trackHeaderRect(0).bottom() + 1;
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge));
            QTest::mouseMove(&timeline, QPoint(140, edge + 24));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 24));
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
        }
        {
            TimelineWidget timeline;
            timeline.setHeightsKey(first.path());
            timeline.setProject(project, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
            QCOMPARE(timeline.allTrackHeight(), 26);
            QJsonObject grown = project;
            QJsonArray tracks = grown.value("tracks").toArray();
            tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
            grown.insert("tracks", tracks);
            timeline.setProject(grown, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26, 26}));
        }
        {
            // Whether the snapshot comes first or not.
            TimelineWidget timeline;
            timeline.setProject(project, 8, 24);
            timeline.setHeightsKey(first.path());
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
        }
        {
            TimelineWidget timeline;
            timeline.setHeightsKey(second.path());
            timeline.setProject(project, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        }
        QSettings settings;
        settings.beginGroup(QStringLiteral("timelineTrackHeights"));
        QStringList paths;
        for (const QString &group : settings.childGroups()) paths << settings.value(group + QStringLiteral("/path")).toString();
        settings.endGroup();
        const QString remembered = QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath());
        QCOMPARE(paths, QStringList{remembered});
        // The editor remembers them for the project it opens, and Medium
        // (the defaults) forgets them.
        MainWindow window{first.path(), QString()};
        QCOMPARE(window.m_timeline->m_heightsPath, remembered);
        window.m_timeline->setProject(project, 8, 24);
        QCOMPARE(trackHeights(*window.m_timeline), (QList<int>{50, 26}));
        window.m_timeline->resetTrackHeights();
        TimelineWidget reopened;
        reopened.setHeightsKey(first.path());
        reopened.setProject(project, 8, 24);
        QCOMPARE(trackHeights(reopened), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        // Left at Mini, it reopens at Mini with Mini checked in the title
        // bar, the remembered heights arriving before any track does.
        const auto checkedHeight = [](const TimelineWidget &timeline) {
            QStringList checked;
            for (const QAction *action : timeline.trackHeightActions())
                if (action->isChecked()) checked << action->objectName();
            return checked;
        };
        reopened.trackHeightActions().at(0)->trigger();
        {
            TimelineWidget atMini;
            atMini.setHeightsKey(first.path());
            atMini.setProject(project, 8, 24);
            QCOMPARE(trackHeights(atMini), QList<int>(2, kMinTrackHeight));
            QCOMPARE(checkedHeight(atMini), QStringList{QStringLiteral("trackHeightMini")});
        }
        {
            MainWindow atMini{first.path(), QString()};
            atMini.setConnection(QStringLiteral("ready"));
            atMini.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", project}});
            QCOMPARE(trackHeights(*atMini.m_timeline), QList<int>(2, kMinTrackHeight));
            QCOMPARE(checkedHeight(*atMini.m_timeline), QStringList{QStringLiteral("trackHeightMini")});
        }
        // A snapshot that removes the one track sized apart leaves every
        // track at one height, and its button checked.
        reopened.trackHeightActions().at(2)->trigger();
        reopened.setTrackHeight(1, 40);
        QVERIFY(checkedHeight(reopened).isEmpty());
        QJsonObject shrunk = project;
        QJsonArray tracks = shrunk.value("tracks").toArray();
        tracks.removeLast();
        shrunk.insert("tracks", tracks);
        reopened.setProject(shrunk, 8, 24);
        QCOMPARE(checkedHeight(reopened), QStringList{QStringLiteral("trackHeightLarge")});
    }

    // From Mini to the tallest, a header's badge and toggles sit inside its
    // row, clear of its top edge and its divider; it is one line (no clip
    // count, no preview) below two lines' height; clips keep their names at
    // every height, show a waveform from a 10px body (rows of 30px) and a
    // filmstrip from a 14px one (rows of 34px).
    void timelineTracksStayLegibleAtEveryHeight() {
        MediaArtwork artwork;
        QImage picture(160, 90, QImage::Format_RGB32);
        picture.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("pic"), picture);
        QImage sound(2048, 48, QImage::Format_ARGB32);
        sound.fill(Qt::white);
        artwork.waves.insert(QStringLiteral("snd"), sound);
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(QJsonObject{{"fps", 24},
            {"sources", QJsonArray{QJsonObject{{"id", "pic"}, {"duration", 4}}, QJsonObject{{"id", "snd"}, {"duration", 4}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{QJsonObject{
                    {"id", "v"}, {"kind", "media"}, {"sourceId", "pic"}, {"name", "Opening shot"}, {"timelineStart", 0.0},
                    {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}}}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{QJsonObject{
                    {"id", "a"}, {"kind", "media"}, {"sourceId", "snd"}, {"name", "Dialogue"}, {"timelineStart", 0.0},
                    {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}}}}}}}}, 6, 24);
        timeline.resize(900, kRulerHeight + 2 * kMaxTrackHeight + kAddLaneHeight + 8);
        timeline.show();
        const EditorTheme &theme = editorTheme();
        // Whether any pixel across a clip's body, near its foot, is `color`.
        const auto bodyShows = [](const QImage &image, const QRect &clip, const QColor &color) {
            for (int x = clip.center().x() - 20; x <= clip.center().x() + 20; ++x)
                if (image.pixelColor(x, clip.bottom() - 3) == color) return true;
            return false;
        };
        for (const int height : {18, 19, 20, 21, 22, 24, 26, 28, 29, 30, 32, 33, 34, 36, 39, 40, 44, 60, 96, 160}) {
            timeline.setAllTrackHeights(height);
            QCOMPARE(trackHeights(timeline), QList<int>(2, height));
            const QImage image = timeline.grab().toImage();
            for (int row = 0; row < 2; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                const QRect badge = timeline.headerBadgeRect(row);
                const QRect output = timeline.headerToggleRect(row, 0);
                const QRect lock = timeline.headerToggleRect(row, 1);
                for (const QRect &control : {badge, output, lock}) {
                    QVERIFY2(control.top() > header.top() && control.bottom() < header.bottom(),
                             qPrintable(QStringLiteral("row %1 at %2px").arg(row).arg(height)));
                    QVERIFY(control.height() >= 12);
                }
                QVERIFY(badge.right() < output.left() && output.right() < lock.left());
                // The divider under the row is whole, past the name.
                QCOMPARE(image.pixelColor(170, header.bottom()), theme.trackDivider);
            }
            const QRect picture = timeline.clipBounds(QStringLiteral("v"));
            const QRect wave = timeline.clipBounds(QStringLiteral("a"));
            QCOMPARE(bodyShows(image, picture, QColor("#e87342")), height >= 34);
            QCOMPARE(bodyShows(image, wave, theme.clipAudio.wave), height >= 30);
            // Names: the clips', the tracks', the badges'; the clip count
            // only on two-line headers.
            TextRecorder recorder;
            QPainter painter(&recorder);
            timeline.paintLanes(painter, timeline.rect());
            timeline.paintHeaders(painter, timeline.rect());
            painter.end();
            for (const char *name : {"Opening shot", "Dialogue", "Video 1", "Audio 1", "V1", "A1"})
                QVERIFY2(recorder.colorOf(QString::fromLatin1(name)).isValid(), name);
            QCOMPARE(recorder.colorOf(QStringLiteral("1 clip")).isValid(), height >= kDetailedTrackHeight);
        }
    }

    // A big edit's track numbers (V100 and up) fit their badges whole, and
    // the toggles after the badges line up down the column.
    void timelineBadgesFitEveryTrackNumber() {
        QStringList kinds;
        for (int index = 0; index < 160; ++index) kinds << (index < 120 ? "video" : "audio");
        TimelineWidget timeline;
        timeline.setProject(trackList(kinds), 8, 24);
        timeline.setAllTrackHeights(kMinTrackHeight);
        timeline.resize(800, 600);
        TextRecorder recorder;
        QPainter painter(&recorder);
        timeline.paintHeaders(painter, timeline.rect());
        painter.end();
        QStringList badges;
        for (const auto &run : recorder.runs)
            if (run.text.size() <= 5 && (run.text.startsWith(QLatin1Char('V')) || run.text.startsWith(QLatin1Char('A')))) badges << run.text;
        QVERIFY2(badges.contains(QStringLiteral("V120")) && badges.contains(QStringLiteral("V100")), qPrintable(badges.join(' ')));
        for (const QString &badge : badges) QVERIFY2(!badge.contains(QChar(0x2026)), qPrintable(badge));
        QFont font = timeline.font();
        font.setPixelSize(10);
        font.setWeight(QFont::DemiBold);
        QVERIFY(timeline.headerBadgeRect(0).width() >= QFontMetrics(font).horizontalAdvance(QStringLiteral("V120")) + 4);
        for (int row = 0; row < timeline.trackRowCount(); row += 37) {
            QCOMPARE(timeline.headerToggleRect(row, 0).left(), timeline.headerToggleRect(0, 0).left());
            QVERIFY(timeline.headerToggleRect(row, 0).left() > timeline.headerBadgeRect(row).right());
        }
    }

    // A full repaint of a 300-track timeline paints the lanes in view, not
    // all of them; and the pointer crossing from clip to clip repaints the
    // clips it leaves and enters, not the view.
    void timelinePaintsOnlyTheTracksInView() {
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineScrollArea scroll;
        scroll.resize(900, 500);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        timeline->setProject(denseTimeline(300, 10.0), 10.0, 24.0);
        timeline->setAllTrackHeights(kMinTrackHeight);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Small enough that the whole timeline could be cached at once.
        QVERIFY(qint64(timeline->width()) * timeline->height() < 6000000);
        scroll.verticalScrollBar()->setValue(2000);
        QCoreApplication::processEvents();
        timeline->invalidateLanes();
        timeline->m_lanesPainted = 0;
        scroll.viewport()->repaint();
        const int inView = scroll.viewport()->height() / kMinTrackHeight + 2;
        QVERIFY2(timeline->m_lanesPainted > 0 && timeline->m_lanesPainted <= inView,
                 qPrintable(QStringLiteral("%1 lanes painted for %2 in view").arg(timeline->m_lanesPainted).arg(inView)));
        // Across three clips of one track: each change of the clip (or the
        // clip's edge) under the pointer repaints the one it leaves and the
        // one it enters, a lane's worth each, never the lanes in view.
        const int y = timeline->mapFrom(scroll.viewport(), QPoint(0, 200)).y();
        QVERIFY(timeline->trackRowAt(y) >= 0);
        QStringList crossed;
        int changes = 0;
        timeline->m_lanesPainted = 0;
        for (int x = kTimelineOrigin + 10; x < kTimelineOrigin + 3 * 216; x += 9) {
            const auto before = std::make_pair(timeline->m_hoverClipId, timeline->m_hoverEdge);
            QTest::mouseMove(timeline, QPoint(x, y));
            QCoreApplication::processEvents();
            changes += before != std::make_pair(timeline->m_hoverClipId, timeline->m_hoverEdge) ? 1 : 0;
            if (!timeline->m_hoverClipId.isEmpty() && !crossed.contains(timeline->m_hoverClipId)) crossed << timeline->m_hoverClipId;
        }
        QVERIFY2(crossed.size() >= 3, qPrintable(crossed.join(QLatin1Char(' '))));
        QVERIFY2(timeline->m_lanesPainted <= 2 * changes && 2 * changes < changes * inView,
                 qPrintable(QStringLiteral("%1 lanes for %2 changes").arg(timeline->m_lanesPainted).arg(changes)));
    }

    // Scrolling moves what is on screen and paints only what scrolls in
    // and the pinned ruler and headers, and what is on screen afterwards is
    // exactly what a full repaint draws: the headers stay in step with
    // their lanes, and the ruler and corner stay pinned.
    void timelineScrollsByMovingWhatIsOnScreen() {
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineScrollArea scroll;
        scroll.resize(900, 500);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        timeline->setProject(denseTimeline(60, 30.0), 30.0, 24.0);
        timeline->setPlayhead(9.5);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        QCoreApplication::processEvents();
        struct PaintMeter final : QObject {
            qint64 pixels = 0;
            bool eventFilter(QObject *, QEvent *event) override {
                if (event->type() == QEvent::Paint)
                    for (const QRect &rect : static_cast<QPaintEvent *>(event)->region()) pixels += qint64(rect.width()) * rect.height();
                return false;
            }
        } meter;
        timeline->installEventFilter(&meter);
        QScrollBar *vertical = scroll.verticalScrollBar();
        QScrollBar *horizontal = scroll.horizontalScrollBar();
        QVERIFY(vertical->maximum() > 400 && horizontal->maximum() > 400);
        const QSize view = scroll.viewport()->size();
        const auto onScreen = [&scroll] { return scroll.screen()->grabWindow(scroll.winId()).toImage(); };
        struct Step {
            int dx;
            int dy;
        };
        for (const Step step : {Step{0, 40}, Step{0, 7}, Step{0, -23}, Step{0, 200}, Step{60, 0}, Step{-15, 0}, Step{3, 0},
                                Step{30, 30}, Step{-41, -9}}) {
            meter.pixels = 0;
            horizontal->setValue(horizontal->value() + step.dx);
            vertical->setValue(vertical->value() + step.dy);
            QCoreApplication::processEvents();
            const qint64 painted = meter.pixels;
            if (step.dx == 0)
                QVERIFY2(painted <= qint64(std::abs(step.dy) + 2 * kRulerHeight + 4) * view.width(),
                         qPrintable(QStringLiteral("%1 px for %2 down").arg(painted).arg(step.dy)));
            if (step.dy == 0)
                QVERIFY2(painted <= qint64(std::abs(step.dx) + 2 * kTimelineLabelWidth + 4) * view.height(),
                         qPrintable(QStringLiteral("%1 px for %2 across").arg(painted).arg(step.dx)));
            const QImage scrolled = onScreen();
            scroll.viewport()->repaint();
            QVERIFY2(scrolled == onScreen(), qPrintable(QStringLiteral("after %1, %2").arg(step.dx).arg(step.dy)));
        }
        timeline->removeEventFilter(&meter);
    }

    // Rows are found by a binary search: correct at every edge, and a
    // hundred times the tracks costs nowhere near a hundred times the time.
    void timelineFindsRowsWithoutWalkingThemAll() {
        const auto timed = [](int count) {
            QStringList kinds;
            for (int index = 0; index < count; ++index) kinds << (index % 3 == 2 ? "audio" : "video");
            TimelineWidget timeline;
            timeline.setProject(trackList(kinds), 8, 24);
            for (int row = 0; row < count; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                if (timeline.trackRowAt(header.top()) != row || timeline.trackRowAt(header.bottom()) != row) return -1.0;
            }
            if (timeline.trackRowAt(kRulerHeight - 1) != -1 || timeline.trackRowAt(timeline.trackHeaderRect(count - 1).bottom() + 1) != -1)
                return -1.0;
            const int span = timeline.trackHeaderRect(count - 1).bottom();
            double best = 1e9;
            qint64 sum = 0;
            for (int run = 0; run < 3; ++run) {
                QElapsedTimer timer;
                timer.start();
                for (int lookup = 0; lookup < 200000; ++lookup) sum += timeline.trackRowAt(kRulerHeight + int((lookup * 7919LL) % span));
                best = std::min(best, timer.nsecsElapsed() / 1e6);
            }
            return sum > 0 ? best : -1.0;
        };
        const double few = timed(30);
        const double many = timed(3000);
        QVERIFY(few >= 0.0 && many >= 0.0);
        // A walk would be some 90 times slower; a search is a few times.
        QVERIFY2(many < 20.0 * std::max(few, 0.2), qPrintable(QStringLiteral("%1 ms for 30 tracks, %2 ms for 3000").arg(few).arg(many)));
    }

    // A dense edit: 48 tracks at Mini, one line each, in both appearances.
    void denseTimelineFollowsTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(denseTimeline(48, 24.0), 24.0, 24.0);
        timeline.setSelectedClip(QStringLiteral("c20_2"));
        timeline.setPlayhead(7.5);
        timeline.trackHeightActions().at(0)->trigger();
        timeline.resize(1440, kRulerHeight + 48 * kMinTrackHeight + kAddLaneHeight + 8);
        timeline.show();
        QCOMPARE(trackHeights(timeline), QList<int>(48, kMinTrackHeight));
        const int playX = kTimelineOrigin + int(std::floor(7.5 * 72.0));
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("timeline-dense-%1.png").arg(theme.dark ? "dark" : "light"))));
            for (int row = 0; row < 48; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                QVERIFY(header.bottom() < image.height());
                // Its stripe is its clips' colour; its line of controls fits it.
                const bool sound = timeline.trackRowLabel(row).startsWith(QLatin1Char('A'));
                QCOMPARE(image.pixelColor(1, header.center().y()), (sound ? theme.clipAudio : theme.clipVideo).header);
                QCOMPARE(image.pixelColor(170, header.bottom()), theme.trackDivider);
                // The playhead's line runs through every row.
                QCOMPARE(image.pixelColor(playX, header.center().y()), theme.playhead);
            }
            QCOMPARE(amberPixels(image), 0);
        }
    }

    void timelineScrubsFromThePointer() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy hovers(&timeline, &TimelineWidget::hoverSeekRequested);
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 144, kTrackHeight / 2));
        QVERIFY(!hovers.isEmpty());
        QCOMPARE(hovers.constLast()[0].toDouble(), 2.0);
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:02.00"));
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 36, kTrackHeight / 2));
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
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 160, y));
        const double start = seconds(timeline.skimLabel());
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 168, y));
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
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Connecting"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Offline"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_timeline->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Offline"));
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

    // A big edit: `count` tracks, two thirds picture and a third sound, each
    // with a clip every three seconds over `seconds`, cut from the fake
    // artwork's sources so filmstrips and waveforms draw.
    static QJsonObject denseTimeline(int count, double seconds) {
        const char *pictures[] = {"wide", "close", "drone"};
        const char *sounds[] = {"wide", "close", "score"};
        QJsonArray sources{
            QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"duration", 20}, {"hasAudio", true}},
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"duration", 20}, {"hasAudio", true}},
            QJsonObject{{"id", "drone"}, {"path", "media/drone-pass.mov"}, {"duration", 20}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}, {"hasAudio", true}}};
        QJsonArray tracks;
        for (int index = 0; index < count; ++index) {
            const bool audio = index % 3 == 2;
            QJsonArray clips;
            for (int n = 0; n * 3.0 < seconds; ++n) {
                // Gaps and lengths vary by track, so the cut reads as an edit.
                const double start = n * 3.0 + (index % 4) * 0.25;
                const double length = 2.0 + ((index + n) % 3) * 0.4;
                QJsonObject clip{{"id", QStringLiteral("c%1_%2").arg(index).arg(n)}, {"kind", "media"},
                                 {"sourceId", audio ? sounds[(index + n) % 3] : pictures[(index + n) % 3]},
                                 {"name", QStringLiteral("Shot %1-%2").arg(index + 1).arg(n + 1)},
                                 {"timelineStart", start}, {"duration", std::min(length, seconds - start)}, {"in", 1.0},
                                 {"out", 1.0 + length}};
                if ((index + n) % 5 == 0)
                    clip.insert("effects", QJsonArray{QJsonObject{{"fx", "color.grade"}, {"version", 1}, {"enabled", true}}});
                clips.append(clip);
            }
            tracks.append(QJsonObject{{"id", QStringLiteral("T%1").arg(index)}, {"type", audio ? "audio" : "video"},
                                      {"name", audio ? QStringLiteral("Sound %1").arg(index / 3 + 1)
                                                     : QStringLiteral("Layer %1").arg(index - index / 3 + 1)},
                                      {"clips", clips}});
        }
        return QJsonObject{{"fps", 24}, {"sources", sources}, {"tracks", tracks}};
    }

    // What a 300-track edit costs: applying a snapshot, painting the panel,
    // scrolling it, skimming across clips, and playing. Prints timings; run
    // with KERF_TIMELINE_BENCH=1 (or the number of tracks).
    void timelineBenchmark() {
        if (qEnvironmentVariableIsEmpty("KERF_TIMELINE_BENCH")) QSKIP("Set KERF_TIMELINE_BENCH=1 to time the timeline.");
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        const int count = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH") > 1 ? qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH") : 300;
        const QJsonObject project = denseTimeline(count, 60.0);
        TimelineScrollArea scroll;
        scroll.resize(1440, 800);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Painted area per paint event, to tell a full repaint from a strip.
        struct PaintMeter final : QObject {
            qint64 pixels = 0;
            int events = 0;
            bool eventFilter(QObject *, QEvent *event) override {
                if (event->type() == QEvent::Paint) {
                    ++events;
                    for (const QRect &rect : static_cast<QPaintEvent *>(event)->region()) pixels += qint64(rect.width()) * rect.height();
                }
                return false;
            }
        } meter;
        timeline->installEventFilter(&meter);
        const auto median = [](QList<double> samples) {
            std::sort(samples.begin(), samples.end());
            return samples.at(samples.size() / 2);
        };
        const auto time = [](const std::function<void()> &work) {
            QElapsedTimer timer;
            timer.start();
            work();
            return timer.nsecsElapsed() / 1e6;
        };
        QList<double> samples;
        for (int run = 0; run < 7; ++run)
            samples << time([&] {
                timeline->setProject(project, 60.0, 24.0);
                QCoreApplication::sendPostedEvents();
                QCoreApplication::processEvents();
            });
        const double apply = median(samples);
        const auto report = [&](const char *what, double ms, qint64 pixels, int events) {
            qInfo("%-28s %8.3f ms  %9lld px painted in %d paint events", what, ms, pixels, events);
        };
        report(qPrintable(QStringLiteral("apply snapshot (%1 tracks)").arg(count)), apply, 0, 0);
        // KERF_TIMELINE_BENCH_HEIGHT puts every track at one height.
        if (const int height = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH_HEIGHT"); height > 0) {
            timeline->setAllTrackHeights(height);
            QCoreApplication::processEvents();
        }
        samples.clear();
        for (int run = 0; run < 21; ++run)
            samples << time([&] {
                timeline->stepAllTrackHeights(run % 2 ? -1 : 1);
                QCoreApplication::processEvents();
            });
        report("step every track's height", median(samples), 0, 0);
        if (const int height = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH_HEIGHT"); height > 0) timeline->setAllTrackHeights(height);
        else timeline->resetTrackHeights();
        QCoreApplication::processEvents();
        samples.clear();
        for (int run = 0; run < 21; ++run) samples << time([&] { scroll.viewport()->repaint(); });
        report("repaint the panel", median(samples), 0, 0);

        QScrollBar *vertical = scroll.verticalScrollBar();
        QVERIFY(vertical->maximum() > 0);
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 1; step <= 60; ++step)
            samples << time([&] {
                vertical->setValue(step * 40);
                QCoreApplication::processEvents();
            });
        report("scroll down 40px", median(samples), meter.pixels / 60, meter.events);

        QScrollBar *horizontal = scroll.horizontalScrollBar();
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 1; step <= 60; ++step)
            samples << time([&] {
                horizontal->setValue(step * 40);
                QCoreApplication::processEvents();
            });
        report("scroll right 40px", median(samples), meter.pixels / 60, meter.events);
        horizontal->setValue(0);
        vertical->setValue(vertical->maximum() / 2);
        QCoreApplication::processEvents();

        // Skimming across clips: each step crosses into the next clip or gap.
        const QPoint origin = timeline->mapFrom(scroll.viewport(), QPoint(0, 0));
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 0; step < 60; ++step)
            samples << time([&] {
                QTest::mouseMove(timeline, origin + QPoint(kTimelineOrigin + 20 + step * 37, 300 + (step % 7) * 11));
                QCoreApplication::processEvents();
            });
        report("skim across clips", median(samples), meter.pixels / 60, meter.events);

        QTest::mouseMove(timeline, origin + QPoint(40, 300));
        QCoreApplication::processEvents();
        samples.clear();
        for (int step = 0; step < 60; ++step)
            samples << time([&] {
                QTest::mouseMove(timeline, origin + QPoint(40 + step % 3, 120 + step * 9));
                QCoreApplication::processEvents();
            });
        report("pointer over the headers", median(samples), 0, 0);

        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int frame = 0; frame < 60; ++frame)
            samples << time([&] {
                timeline->setPlayhead(frame / 24.0);
                QCoreApplication::processEvents();
            });
        report("play a frame", median(samples), meter.pixels / 60, meter.events);
        timeline->removeEventFilter(&meter);
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

        // Every state's words read at 4.5:1 on the status bar, in both
        // appearances; "Not shared" is the only sign a project is not.
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        QJsonObject live = status;
        live.insert("live", true);
        const QList<QJsonObject> events{
            QJsonObject{{"type", "collab"}, {"status", status}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{{"incoming", QJsonArray{}}, {"pushed", false}}}},
            QJsonObject{{"type", "collab"}, {"status", live}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "offline"}},
            QJsonObject{{"type", "collab"}, {"status", conflicted}},
            QJsonObject{{"type", "collab"}, {"status", local}}};
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            QSet<QString> states;
            for (const QJsonObject &event : events) {
                window.handleEvent(event);
                const QString state = window.m_syncChip->property("state").toString();
                states.insert(state);
                const QColor ink = window.m_syncChip->palette().color(QPalette::ButtonText);
                const qreal ratio = contrastRatio(ink, editorTheme().statusBar);
                QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1: %2 on %3: %4")
                                                      .arg(state, ink.name(), editorTheme().statusBar.name())
                                                      .arg(ratio, 0, 'f', 2)));
            }
            QCOMPARE(states, (QSet<QString>{"idle", "syncing", "live", "failed", "conflict", "local"}));
        }
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
        QVERIFY(window.findChild<QLabel *>(QStringLiteral("divergedTitle"))->text().contains(QStringLiteral("project.kerf changed on disk")));
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
        QCOMPARE(bin->badge(QStringLiteral("m1")), QStringLiteral("Not downloaded"));
        QCOMPARE(bin->badge(QStringLiteral("m2")), QStringLiteral("External"));
        QCOMPARE(bin->badge(QStringLiteral("m3")), QStringLiteral("Offline"));
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
        QPoint firstAmber;
        {
            QImage probe(40, 40, QImage::Format_RGB32);
            probe.fill(Qt::white);
            QPainter painter(&probe);
            painter.fillRect(QRect(0, 0, 1, 40), QColor(0xf0, 0xb4, 0x29));
            QCOMPARE(amberPixels(probe), 0);
            painter.fillRect(QRect(10, 10, 20, 20), QColor(0xf0, 0xb4, 0x29));
            QVERIFY(amberPixels(probe) > 0);
        }
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
                QVERIFY2(amberPixels(image, &firstAmber) == 0,
                         qPrintable(QStringLiteral("amber area in %1-%2 at %3,%4")
                                        .arg(QString::fromLatin1(pageNames[page]), schemeName)
                                        .arg(firstAmber.x())
                                        .arg(firstAmber.y())));
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
                    // Its chevron, the only sign of whether they are open,
                    // reads on the content (3:1), closed and open.
                    QCOMPARE(ink(launcher.m_detailsToggle->icon()), launcher.theme().secondary.rgb());
                    launcher.m_detailsToggle->click();
                    QVERIFY(!launcher.m_detailsBody->isHidden());
                    QCOMPARE(ink(launcher.m_detailsToggle->icon()), launcher.theme().secondary.rgb());
                    launcher.m_git->setChecked(true);
                    launcher.m_name->setFocus();
                    QCoreApplication::processEvents();
                    image = launcher.grab().toImage();
                    QVERIFY2(amberPixels(image, &firstAmber) == 0,
                             qPrintable(QStringLiteral("amber area in new-details-%1 at %2,%3")
                                            .arg(schemeName)
                                            .arg(firstAmber.x())
                                            .arg(firstAmber.y())));
                    save(image, QStringLiteral("new-details-%1").arg(schemeName));
                    // The remote field's placeholder reads at 4.5:1 in the
                    // editor's placeholder colour, and dims when the field is off.
                    const LauncherTheme &look = launcher.theme();
                    launcher.m_remote->ensurePolished();
                    QVERIFY(launcher.m_remote->isEnabled() && launcher.m_remote->text().isEmpty());
                    for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive})
                        QCOMPARE(launcher.m_remote->palette().color(group, QPalette::PlaceholderText).rgb(),
                                 makeEditorTheme(dark, look.accent).placeholderText.rgb());
                    QVERIFY(contrastRatio(launcher.m_remote->palette().color(QPalette::Active, QPalette::PlaceholderText),
                                          look.field) >= 4.5);
                    QCOMPARE(launcher.m_remote->palette().color(QPalette::Disabled, QPalette::PlaceholderText).rgb(),
                             look.disabledText.rgb());
                    // Its frame-rate pop-up opens with every choice showing.
                    QVERIFY(popupShowsEveryRow(launcher.m_rate));
                }
                launcher.close();
            }
        }
        ProjectLauncher::clearRecentProjects();
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
    }

    // A chosen recent project's text reads on its selection (4.5:1), in
    // both appearances: a missing project's name on the grey of a list
    // without focus, and the details on the focused list's blue. The colours
    // are read from the pens the rows are painted with.
    void launcherRowsReadWhenChosen() {
        const auto clean = qScopeGuard([] {
            ProjectLauncher::clearRecentProjects();
            QSettings().remove(QStringLiteral("launcher"));
            QSettings().remove(QStringLiteral("projects"));
        });
        QTemporaryDir root;
        QVERIFY(root.isValid());
        const QString present = QDir(root.path()).filePath(QStringLiteral("Harbor Teaser"));
        QVERIFY(QDir().mkpath(present));
        const QString missing = QDir(root.path()).filePath(QStringLiteral("Gone Project"));
        ProjectLauncher::clearRecentProjects();
        ProjectLauncher::rememberProject(present);
        ProjectLauncher::rememberProject(missing);
        for (const auto scheme : {ProjectLauncher::ColorScheme::Light, ProjectLauncher::ColorScheme::Dark}) {
            QSettings().remove(QStringLiteral("launcher"));
            ProjectLauncher launcher;
            launcher.setColorScheme(scheme);
            launcher.m_inspectProject = [](const QJsonObject &request) {
                const QString path = request.value("path").toString();
                return QJsonObject{{"path", path}, {"exists", QFileInfo(path).isDir()}, {"name", QFileInfo(path).fileName()},
                                   {"branch", "main"}};
            };
            launcher.reloadRecent();
            const LauncherTheme &t = launcher.theme();
            QListWidget *list = launcher.m_recent;
            QCOMPARE(list->count(), 2);
            // The colour `text` is painted in on row `row`, chosen.
            const auto painted = [list](int row, const QString &text) {
                QStyleOptionViewItem option;
                option.initFrom(list);
                option.widget = list;
                option.font = list->font();
                option.rect = QRect(0, 0, 600, 52);
                option.state |= QStyle::State_Selected;
                TextRecorder recorder;
                QPainter painter(&recorder);
                list->itemDelegate()->paint(&painter, option, list->model()->index(row, 0));
                painter.end();
                return recorder.colorOf(text);
            };
            // Missing, chosen, in a list without focus (as when the sidebar has it).
            QVERIFY(!list->hasFocus());
            QCOMPARE(list->item(0)->text().section(QLatin1Char('\n'), 0, 0), QStringLiteral("Gone Project"));
            const QColor name = painted(0, QStringLiteral("Gone Project"));
            QVERIFY(name.isValid());
            QVERIFY2(contrastRatio(laidOn(t.selectionInactive, name), t.selectionInactive) >= 4.5, qPrintable(name.name()));
            // Present, chosen, in the focused list.
            launcher.showPage(ProjectLauncher::Recent);
            launcher.show();
            QVERIFY(QTest::qWaitForWindowExposed(&launcher));
            launcher.activateWindow();
            list->setFocus();
            QTRY_VERIFY(list->hasFocus());
            const QColor branch = painted(1, QStringLiteral("main"));
            QVERIFY(branch.isValid());
            QVERIFY2(contrastRatio(laidOn(t.selection, branch), t.selection) >= 4.5, qPrintable(branch.name(QColor::HexArgb)));
            launcher.close();
        }
    }

    // Whether `combo`'s menu, opened, shows every choice whole: no scroll
    // arrows and nothing to scroll.
    static bool popupShowsEveryRow(QComboBox *combo) {
        combo->showPopup();
        QAbstractItemView *list = combo->view();
        bool scrollers = false;
        for (const QWidget *child : list->window()->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
            scrollers = scrollers || (child->inherits("QComboBoxPrivateScroller") && child->isVisible());
        const bool fits = !scrollers && list->verticalScrollBar()->maximum() == 0;
        combo->hidePopup();
        return fits;
    }

    // Whether `combo`, in `shot` of its window, is drawn as macOS draws a
    // pop-up button: a push button's bezel, filled with `button` and lit from
    // above (its bottom edge darker than its top), not a field's even edge.
    static bool popUpHasButtonBezel(const QImage &shot, const QComboBox *combo, const QColor &button) {
        const qreal ratio = shot.devicePixelRatio();
        const QRect bezel(combo->mapTo(combo->window(), QPoint()), combo->size());
        const auto at = [&](int x, int y) { return shot.pixelColor(QPoint(x, y) * ratio); };
        return at(bezel.left() + 4, bezel.center().y()).rgb() == button.rgb() &&
               at(bezel.center().x(), bezel.bottom()).lightness() + 16 < at(bezel.center().x(), bezel.top()).lightness();
    }

    // Saturated amber or gold, as the editor's old accent was.
    static bool isAmber(const QColor &color) {
        return color.hue() >= 30 && color.hue() <= 52 && color.saturation() > 140 && color.value() > 150;
    }
    // How many amber areas `image` has, and where the first is. A pixel
    // counts only when its eight neighbours are amber too, so the coloured
    // fringes of subpixel-antialiased text (as on Linux) are not counted.
    static int amberPixels(const QImage &image, QPoint *first = nullptr) {
        int count = 0;
        for (int y = 1; y + 1 < image.height(); y += 2)
            for (int x = 1; x + 1 < image.width(); x += 2) {
                bool area = true;
                for (int dy = -1; dy <= 1 && area; ++dy)
                    for (int dx = -1; dx <= 1 && area; ++dx) area = isAmber(image.pixelColor(x + dx, y + dy));
                if (area && count++ == 0 && first) *first = QPoint(x, y);
            }
        return count;
    }
    // The most contrast any pixel of `area` in `image` has with `background`.
    static qreal highestContrast(const QImage &image, const QRect &area, const QColor &background) {
        qreal best = 1.0;
        const QRect inside = area.intersected(image.rect());
        for (int y = inside.top(); y <= inside.bottom(); ++y)
            for (int x = inside.left(); x <= inside.right(); ++x)
                best = std::max(best, contrastRatio(image.pixelColor(x, y), background));
        return best;
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
    // `top` as it shows laid on the opaque `under`.
    static QColor laidOn(const QColor &under, QColor top) {
        const qreal alpha = top.alphaF();
        top.setAlpha(255);
        return mixColor(under, top, alpha);
    }
    // The colour `icon`'s symbol is drawn in: its most opaque pixel's.
    static QRgb ink(const QIcon &icon, QIcon::Mode mode = QIcon::Normal, QIcon::State state = QIcon::Off) {
        const QImage image = icon.pixmap(QSize(20, 20), 2.0, mode, state).toImage();
        QColor best(0, 0, 0, 0);
        for (int y = 0; y < image.height(); ++y)
            for (int x = 0; x < image.width(); ++x)
                if (image.pixelColor(x, y).alpha() > best.alpha()) best = image.pixelColor(x, y);
        return best.rgb();
    }
    // macOS's accents in light and dark: blue, purple, pink, red, orange,
    // yellow, green, and graphite.
    static QList<QColor> macAccents() {
        return {QColor("#007aff"), QColor("#0a84ff"), QColor("#953d96"), QColor("#bf5af2"), QColor("#f74f9e"), QColor("#ff375f"),
                QColor("#e0383e"), QColor("#ff453a"), QColor("#f7821b"), QColor("#ff9f0a"), QColor("#ffc600"), QColor("#ffd60a"),
                QColor("#62ba46"), QColor("#32d74b"), QColor("#8c8c8c"), QColor("#98989d")};
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

    // The accent as text (the transports' timecodes, the timeline's corner
    // time, the sync chip) reads at 4.5:1 on each surface it is drawn on,
    // whichever accent System Settings has, in both appearances.
    void accentTextReadsOnEverySurfaceItIsDrawnOn() {
        for (const bool dark : {false, true})
            for (const QColor &accent : macAccents()) {
                const EditorTheme theme = makeEditorTheme(dark, accent);
                for (const QColor &surface : {theme.panel, theme.rulerCorner, theme.statusBar}) {
                    const qreal ratio = contrastRatio(theme.accentText, surface);
                    QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1 on %2 in %3: %4")
                                                          .arg(theme.accentText.name(), surface.name(),
                                                               dark ? QStringLiteral("dark") : QStringLiteral("light"))
                                                          .arg(ratio, 0, 'f', 2)));
                }
                // So does the Source monitor's marked range, at 3:1 on its panel.
                QVERIFY2(contrastRatio(theme.inOutRange, theme.panel) >= 3.0, qPrintable(accent.name()));
            }
        // The dark default reads on the timeline's corner with room to spare.
        QVERIFY(contrastRatio(makeEditorTheme(true, QColor("#0a84ff")).accentText, QColor("#232325")) >= 5.0);
    }

    // Colours drawn one on another, each with the ratio it must reach there
    // (4.5:1 for text, 3:1 for symbols, edges and handles), in both
    // appearances; a translucent one is laid on what is under it first.
    // Pairs that come from the accent are checked with each of macOS's.
    void themeColorsReadWhereTheyAreDrawn_data() {
        QTest::addColumn<QColor>("foreground");
        QTest::addColumn<QColor>("background");
        QTest::addColumn<qreal>("ratio");
        struct Pair {
            QByteArray what;
            QColor foreground;
            QColor background;
            qreal ratio;
        };
        for (const bool dark : {false, true})
            for (const QColor &accent : macAccents()) {
                const EditorTheme t = makeEditorTheme(dark, accent);
                const LauncherTheme l = launcherTheme(dark, accent);
                // Secondary text on a selected row reads as text wherever
                // white does (whether white does on every accent is the
                // accent's question, not this token's).
                std::vector<Pair> pairs{
                    {"chosen mask tool's symbol on its wash", t.accentText, t.accentTint, 3.0},
                    {"chosen mask tool's symbol on its wash, held", t.accentText, t.accentTintPressed, 3.0},
                    {"selected mask's name on its wash", t.buttonText, t.accentTint, 4.5},
                    {"selected mask's name on its wash, held", t.buttonText, t.accentTintPressed, 4.5},
                    {"selected mask's edge on the panel", t.accentText, t.panel, 3.0},
                    {"chosen mode's edge on a title strip", t.accentText, t.panelTitle, 3.0},
                    {"open conversation switch's edge on the toolbar", t.accentText, t.chrome, 3.0},
                    {"playhead's head on the ruler's top", t.playhead, t.ruler, 3.0},
                    {"playhead's head on the ruler's foot", t.playhead, t.rulerBottom, 3.0},
                    {"playhead's line on its halo", t.playhead, t.playheadHalo, 3.0},
                    {"secondary text on a selected row", t.onAccentSecondary, t.selection,
                     std::min(4.5, contrastRatio(Qt::white, t.selection))},
                    {"launcher: secondary text on a selected row", l.onAccentSecondary, l.selection,
                     std::min(4.5, contrastRatio(Qt::white, l.selection))},
                };
                if (accent == systemAccent(dark)) {
                    pairs.insert(pairs.end(), {
                        {"Model field under the pointer", t.text, laidOn(t.field, t.hover), 4.5},
                        {"Model field under the pointer while sending", t.text, laidOn(t.assistantSurface, t.hover), 4.5},
                        {"disclosure chevron on a dialog", t.textSecondary, t.dialog, 3.0},
                        {"disclosure chevron on a dialog's group", t.textSecondary, laidOn(t.dialog, t.group), 3.0},
                        {"disclosure chevron on a panel", t.textSecondary, t.panel, 3.0},
                        {"disclosure chevron on a well", t.textSecondary, t.well, 3.0},
                        {"scroll bar handle on a panel", t.scrollHandle, t.panel, 3.0},
                        {"scroll bar handle on a well", t.scrollHandle, t.well, 3.0},
                        {"scroll bar handle on a field", t.scrollHandle, t.field, 3.0},
                        {"scroll bar handle on a dialog", t.scrollHandle, t.dialog, 3.0},
                        {"added clip's name on its band", t.assistantText, laidOn(t.trackLane, t.proposalAddFill), 4.5},
                        {"added clip's name on its band over a locked lane's hatch", t.assistantText,
                         laidOn(laidOn(t.trackLane, t.trackLaneLocked), t.proposalAddFill), 4.5},
                        {"skimmed playhead's head on the ruler's top", t.playheadIdle, t.ruler, 3.0},
                        {"skimmed playhead's head on the ruler's foot", t.playheadIdle, t.rulerBottom, 3.0},
                        {"skimmed playhead's line on its halo", t.playheadIdle, t.playheadHalo, 3.0},
                        {"You on the user's message", t.messageUserRole, t.messageUser, 4.5},
                        {"unreadable preset on the preset list", t.error, t.well, 4.5},
                        {"launcher: Details chevron on the content", l.secondary, l.content, 3.0},
                        {"launcher: scroll bar handle on the content", l.scrollHandle, l.content, 3.0},
                        {"launcher: scroll bar handle on a group", l.scrollHandle, l.group, 3.0},
                        {"launcher: scroll bar handle on the sidebar", l.scrollHandle, l.sidebar, 3.0},
                        {"launcher: missing project on an inactive selection", l.secondaryOnSelection, l.selectionInactive, 4.5},
                    });
                    for (const QByteArray marker : {"red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white", ""}) {
                        const QColor color = t.marker(QString::fromLatin1(marker));
                        const QByteArray name = marker.isEmpty() ? QByteArray("uncoloured") : marker;
                        pairs.push_back({name + " marker on the ruler's top", color, t.ruler, 3.0});
                        pairs.push_back({name + " marker on the ruler's foot", color, t.rulerBottom, 3.0});
                    }
                }
                for (const Pair &pair : pairs)
                    QTest::addRow("%s (%s), %s, %s", pair.what.constData(), qPrintable(pair.foreground.name(QColor::HexArgb)),
                                  dark ? "dark" : "light", qPrintable(accent.name()))
                        << pair.foreground << pair.background << pair.ratio;
            }
    }
    void themeColorsReadWhereTheyAreDrawn() {
        QFETCH(QColor, foreground);
        QFETCH(QColor, background);
        QFETCH(qreal, ratio);
        const QColor drawn = laidOn(background, foreground);
        const qreal measured = contrastRatio(drawn, background);
        QVERIFY2(measured >= ratio, qPrintable(QStringLiteral("%1 on %2: %3, under %4")
                                                   .arg(drawn.name(), background.name())
                                                   .arg(measured, 0, 'f', 2)
                                                   .arg(ratio, 0, 'f', 2)));
    }

    // States meant to differ still do: the skimmed playhead is a grey that
    // reads clearly less than the playhead itself wherever either is drawn
    // (the ruler for their heads, their solid halo for their lines), for
    // every accent; and a scroll bar's handle under the pointer or held
    // reads more than at rest.
    void quietAndActiveStatesStayApart() {
        for (const bool dark : {false, true}) {
            for (const QColor &accent : macAccents()) {
                const EditorTheme t = makeEditorTheme(dark, accent);
                QVERIFY(t.playheadIdle.hsvSaturation() < 20);
                QCOMPARE(t.playheadHalo.alpha(), 255);
                for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
                    QVERIFY2(contrastRatio(t.playheadIdle, surface) + 0.35 <= contrastRatio(t.playhead, surface),
                             qPrintable(QStringLiteral("%1 and %2 on %3, %4").arg(t.playheadIdle.name(), t.playhead.name(),
                                                                              surface.name(), accent.name())));
            }
            const EditorTheme t = makeEditorTheme(dark, systemAccent(dark));
            // The system's own blue stays the playhead's, as it is.
            QCOMPARE(t.playhead.rgb(), systemAccent(dark).rgb());
            const LauncherTheme l = launcherTheme(dark, systemAccent(dark));
            const auto stronger = [](const QColor &rest, const QColor &active, const QColor &surface) {
                return contrastRatio(laidOn(surface, active), surface) > contrastRatio(laidOn(surface, rest), surface) + 1.0;
            };
            for (const QColor &surface : {t.panel, t.well, t.field, t.dialog})
                QVERIFY2(stronger(t.scrollHandle, t.scrollHandleHover, surface), qPrintable(surface.name()));
            for (const QColor &surface : {l.content, l.group, l.sidebar})
                QVERIFY2(stronger(l.scrollHandle, l.scrollHandleHover, surface), qPrintable(surface.name()));
        }
        // Both sheets give the stronger handle to a held one as well as a hovered one.
        QVERIFY(editorStyleSheet(editorTheme()).contains(QStringLiteral("QScrollBar::handle:vertical:pressed")));
        QVERIFY(launcherStyleSheet(launcherTheme(true, systemAccent(true))).contains(QStringLiteral("QScrollBar::handle:vertical:pressed")));
    }

    // The chosen mask tool and the selected mask stand out from the others,
    // at rest, under the pointer and held (darkening while held, as every
    // bezel does), in Effect Controls and in the Program panel's title
    // strip alike; and the Assistant's Model field reads on its hover wash,
    // in both appearances.
    void chosenMasksAndToolsStandOut() {
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            QSettings().remove(QStringLiteral("workspace"));
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            const auto mask = [](const char *name) {
                return QJsonObject{{"name", name}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "add"},
                                   {"inverted", false}, {"params", QJsonObject{}}};
            };
            window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 2}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1920}, {"height", 1080}, {"maskModes", QJsonArray{"add", "subtract", "none"}},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 2}}}},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"masks", QJsonArray{mask("k1"), mask("k2")}}}}}}}}}}});
            window.selectClip(QStringLiteral("c1"));
            window.m_effectControls->selectMask(QStringLiteral("k1"));
            window.m_workspace->panel(QStringLiteral("effectControls"))->toggleView(true);
            window.m_workspace->panel(QStringLiteral("effectControls"))->setAsCurrentTab();
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // The bezel's fill, clear of its edge and its symbol or label.
            const auto fill = [](QToolButton *button) {
                const QImage image = button->grab().toImage();
                return image.pixelColor(QPoint(3, button->height() / 2) * image.devicePixelRatio()).rgb();
            };
            // Under the pointer, and held.
            const auto hovered = [&fill](QToolButton *button) {
                button->setAttribute(Qt::WA_UnderMouse, true);
                const QRgb color = fill(button);
                button->setAttribute(Qt::WA_UnderMouse, false);
                return color;
            };
            const auto held = [&fill](QToolButton *button) {
                button->setDown(true);
                const QRgb color = fill(button);
                button->setDown(false);
                return color;
            };

            // The chosen tool is an accent wash, its symbol the accent, which
            // reads on the wash; the others keep the plain bezel and glyph.
            auto *rectangle = window.findChild<QAction *>(QStringLiteral("maskRectangle"));
            QVERIFY(rectangle && rectangle->isEnabled());
            rectangle->trigger();
            QVERIFY(rectangle->isChecked());
            QToolButton *chosen = nullptr;
            QToolButton *other = nullptr;
            for (QToolButton *tool : window.m_effectControls->findChildren<QToolButton *>(QStringLiteral("maskTool")))
                (tool->defaultAction() == rectangle ? chosen : other) = tool;
            QVERIFY(chosen && other && chosen->isVisible() && chosen->isChecked() && !other->isChecked());
            QCOMPARE(fill(other), theme.button.rgb());
            QCOMPARE(hovered(other), theme.buttonHover.rgb());
            QCOMPARE(held(other), theme.buttonPressed.rgb());
            for (const QRgb wash : {fill(chosen), hovered(chosen)}) QCOMPARE(wash, theme.accentTint.rgb());
            QCOMPARE(held(chosen), theme.accentTintPressed.rgb());
            QVERIFY(qGray(theme.accentTintPressed.rgb()) < qGray(theme.accentTint.rgb()));
            QCOMPARE(ink(chosen->icon(), QIcon::Normal, QIcon::On), theme.accentText.rgb());
            QCOMPARE(ink(other->icon()), theme.glyph.rgb());
            QVERIFY(contrastRatio(theme.accentText, theme.accentTint) >= 3.0);
            // The Program panel's title strip carries the same tools, and
            // shows the chosen one the same way.
            QToolButton *strip = nullptr;
            for (QToolButton *button : window.m_workspace->panel(QStringLiteral("program"))
                                           ->dockAreaWidget()->findChildren<QToolButton *>(QStringLiteral("maskRectangle")))
                if (button->defaultAction() == rectangle) strip = button;
            QVERIFY(strip && strip->isVisible() && strip->isChecked());
            for (const QRgb wash : {fill(strip), hovered(strip)}) QCOMPARE(wash, theme.accentTint.rgb());
            QCOMPARE(held(strip), theme.accentTintPressed.rgb());
            QCOMPARE(ink(strip->icon(), QIcon::Normal, QIcon::On), theme.accentText.rgb());
            rectangle->trigger();
            QVERIFY(!rectangle->isChecked());
            QCOMPARE(fill(chosen), theme.button.rgb());

            // So is the selected mask's name; the other's is the plain bezel.
            QToolButton *selected = nullptr;
            QToolButton *unselected = nullptr;
            for (QToolButton *name : window.m_effectControls->findChildren<QToolButton *>(QStringLiteral("maskName")))
                (name->text() == QStringLiteral("k1") ? selected : unselected) = name;
            QVERIFY(selected && unselected && selected->isVisible() && selected->isChecked() && !unselected->isChecked());
            for (const QRgb wash : {fill(selected), hovered(selected)}) QCOMPARE(wash, theme.accentTint.rgb());
            QCOMPARE(held(selected), theme.accentTintPressed.rgb());
            QCOMPARE(fill(unselected), theme.button.rgb());
            QVERIFY(theme.accentTint.rgb() != theme.button.rgb());

            // The Model field's text is drawn in one colour, hovered or not,
            // so that colour reads on the hover wash.
            QLineEdit *model = window.m_modelSelect->lineEdit();
            QVERIFY(model);
            model->ensurePolished();
            const QColor modelText = model->palette().color(QPalette::Active, QPalette::Text);
            QCOMPARE(modelText.rgb(), theme.text.rgb());
            QVERIFY(contrastRatio(modelText, laidOn(theme.field, theme.hover)) >= 4.5);
            window.close();
        }
    }

    // A colour well's swatch is ringed in a grey that reads at 3:1 against
    // white and black alike, so a white swatch shows on the light bezel and
    // a black one on the dark, without a light halo on the dark bezel.
    void colorWellsRingWhiteAndBlackSwatches() {
        QPushButton well;
        well.setIconSize(QSize(32, 12));
        for (const bool dark : {false, true}) {
            const QColor value = dark ? QColor(Qt::black) : QColor(Qt::white);
            const QColor bezel = makeEditorTheme(dark, systemAccent(dark)).button;
            EffectControlsPanel::setPropertyValue(&well, value.name());
            const QImage swatch = well.icon().pixmap(well.iconSize(), 2.0).toImage();
            QCOMPARE(swatch.size(), well.iconSize() * 2);
            QCOMPARE(swatch.pixelColor(swatch.width() / 2, swatch.height() / 2).rgb(), value.rgb());
            // The ring's outer half, solid at twice the size, as it shows on the bezel.
            const QColor ring = laidOn(bezel, swatch.pixelColor(0, swatch.height() / 2));
            QVERIFY2(contrastRatio(ring, value) >= 3.0, qPrintable(ring.name()));
            // It shows on the light bezel, and on the dark one keeps to the
            // bezel's own grey rather than ringing the swatch in light.
            if (!dark) QVERIFY2(contrastRatio(ring, bezel) >= 3.0, qPrintable(ring.name()));
            else QVERIFY2(contrastRatio(ring, bezel) < 1.3 && ring.lightness() < bezel.lightness() + 12, qPrintable(ring.name()));
        }
    }

    // An unreadable preset's red is the appearance's own after a switch,
    // not the one it was listed in.
    void unreadablePresetsTurnRedInEitherAppearance() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        setEditorColorScheme(ColorScheme::Light);
        PresetsPanel panel;
        panel.setLibrary(QJsonObject{{"presets", QJsonArray{
            QJsonObject{{"path", "presets/old.kerfpreset"}, {"name", "old"}, {"error", "line 3: unknown statement"}},
            QJsonObject{{"path", "presets/intro.kerfpreset"}, {"name", "Intro"}, {"slots", QJsonArray{}}}}}}, QDir::tempPath());
        QListWidgetItem *unreadable = panel.m_list->item(0);
        QListWidgetItem *readable = panel.m_list->item(1);
        QCOMPARE(unreadable->foreground().color(), editorTheme().error);
        for (const ColorScheme scheme : {ColorScheme::Dark, ColorScheme::Light}) {
            setEditorColorScheme(scheme);
            QCOMPARE(unreadable->foreground().color(), editorTheme().error);
            QVERIFY(contrastRatio(unreadable->foreground().color(), editorTheme().well) >= 4.5);
            QVERIFY(!readable->data(Qt::ForegroundRole).isValid());
        }
    }

    // A clip a proposal adds is named on a solid band, so its name reads
    // over a clip already in the track (an overwrite's, a replacement's) as
    // over the bare lane.
    void proposedClipNamesReadOverClipsAlreadyThere() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        const QJsonObject wide{{"id", "a"}, {"kind", "media"}, {"timelineStart", 0.0}, {"duration", 8.0}};
        const QJsonObject inserted{{"id", "n"}, {"kind", "media"}, {"name", "Insert"}, {"timelineStart", 2.0}, {"duration", 4.0}};
        const auto timeline = [](const QJsonArray &clips) {
            return QJsonObject{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", clips}}}}};
        };
        TimelineWidget widget;
        widget.setProject(timeline(QJsonArray{wide}), 10, 24);
        widget.setProposal(timeline(QJsonArray{wide, inserted}));
        widget.resize(1000, 200);
        widget.show();
        const QRect existing = widget.clipBounds(QStringLiteral("a"));
        QVERIFY(existing.width() > 200);
        // Near the added clip's right end (6 s), clear of its name and its dashes,
        // inside its name's band, over the existing clip's name bar.
        const QPoint band(existing.left() + existing.width() * 3 / 4 - 10, existing.top() + 7);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = widget.grab().toImage();
            const QColor drawn = image.pixelColor(band * image.devicePixelRatio());
            const QColor expected = laidOn(theme.trackLane, theme.proposalAddFill);
            QVERIFY2(std::abs(drawn.red() - expected.red()) <= 2 && std::abs(drawn.green() - expected.green()) <= 2 &&
                         std::abs(drawn.blue() - expected.blue()) <= 2,
                     qPrintable(QStringLiteral("%1, not %2").arg(drawn.name(), expected.name())));
            QVERIFY(contrastRatio(theme.assistantText, drawn) >= 4.5);
        }

        // On a locked track the band keeps the lane's hatch, so the lock
        // shows under the name too, and the name still reads on it.
        QJsonObject locked = timeline(QJsonArray{wide});
        QJsonObject lockedProposal = timeline(QJsonArray{wide, inserted});
        for (QJsonObject *object : {&locked, &lockedProposal}) {
            QJsonArray tracks = object->value("tracks").toArray();
            QJsonObject track = tracks.at(0).toObject();
            track.insert("locked", true);
            tracks.replace(0, track);
            object->insert("tracks", tracks);
        }
        widget.setProject(locked, 10, 24);
        widget.setProposal(lockedProposal);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = widget.grab().toImage();
            const auto near = [](const QColor &a, const QColor &b) {
                return std::abs(a.red() - b.red()) <= 2 && std::abs(a.green() - b.green()) <= 2 && std::abs(a.blue() - b.blue()) <= 2;
            };
            const QColor bare = laidOn(theme.trackLane, theme.proposalAddFill);
            const QColor hatch = laidOn(laidOn(theme.trackLane, theme.trackLaneLocked), theme.proposalAddFill);
            QVERIFY(!near(bare, hatch));
            // Along the band, clear of the name: the hatch's lines cross it.
            int lines = 0;
            int plain = 0;
            for (int x = band.x() - 16; x <= band.x() + 16; ++x) {
                const QColor drawn = image.pixelColor(QPoint(x, band.y()) * image.devicePixelRatio());
                lines += near(drawn, hatch) ? 1 : 0;
                plain += near(drawn, bare) ? 1 : 0;
            }
            QVERIFY2(lines >= 3 && plain >= 3, qPrintable(QStringLiteral("%1 hatch, %2 plain").arg(lines).arg(plain)));
            QVERIFY(contrastRatio(theme.assistantText, hatch) >= 4.5);
        }
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
            // Clip labels read on their headers, at the base of the name bar
            // and at its lightened top, where the label sits.
            for (const ClipKind kind : {ClipKind::Video, ClipKind::Audio, ClipKind::Music, ClipKind::Generator, ClipKind::Caption}) {
                QVERIFY(contrastRatio(theme.clip(kind).label, theme.clip(kind).header) >= 4.5);
                QVERIFY(contrastRatio(theme.clip(kind).label, theme.clip(kind).header.lighter(108)) >= 4.5);
            }
            // A caption clip's body stands off its lane.
            QVERIFY(contrastRatio(theme.clipCaption.fill, theme.trackLane) >= 1.12);
            // Every marker reads as a mark on the ruler.
            for (const char *marker : {"red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white"})
                QVERIFY2(contrastRatio(theme.marker(QString::fromLatin1(marker)), theme.ruler) >= 3.0, marker);
            // Secondary text reads on the grey window and dialogs too, and
            // Kerf's suggestion chips on their purple surface.
            QVERIFY(contrastRatio(theme.textSecondary, theme.dialog) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.panelTitle) >= 4.5);
            QVERIFY(contrastRatio(theme.assistantText, theme.assistantSurface) >= 4.5);
            QVERIFY(contrastRatio(theme.assistantText, theme.assistantHover) >= 4.5);
            QVERIFY(contrastRatio(theme.badgeErrorText, theme.badgeError) >= 4.5);
            // In dark, the current tab is lighter than the strip, as in light.
            QVERIFY(theme.panelTitleActive.lightness() > theme.panelTitle.lightness());
            // The accent as text reads on each surface it is drawn on; see
            // accentTextReadsOnEverySurfaceItIsDrawnOn for every accent.
            for (const QColor &surface : {theme.panel, theme.rulerCorner, theme.statusBar})
                QVERIFY2(contrastRatio(theme.accentText, surface) >= 4.5, qPrintable(surface.name()));
            // A track's mute and lock letters read on their toggles.
            QVERIFY(contrastRatio(theme.trackToggleGlyph, theme.trackToggle) >= 4.5);
            // An unchecked check box's edge reads (3:1) on the panels and
            // dialogs it sits on, and on a dialog's grouped boxes.
            const auto over = [](const QColor &under, QColor top) {
                const qreal alpha = top.alphaF();
                top.setAlpha(255);
                return mixColor(under, top, alpha);
            };
            for (const QColor &surface : {theme.panel, theme.dialog, over(theme.dialog, theme.group)})
                QVERIFY2(contrastRatio(theme.checkBorder, surface) >= 3.0, qPrintable(surface.name()));
            QVERIFY(contrastRatio(theme.assistantText, theme.chrome) >= 4.5);
            QVERIFY(contrastRatio(theme.tabInactiveText, theme.panelTitle) >= 4.5);
            QVERIFY(contrastRatio(theme.trackHeaderDetail, theme.trackHeader) >= 4.5);
            QVERIFY(contrastRatio(theme.tileEmptyText, theme.tileEmpty) >= 4.5);
            QVERIFY(contrastRatio(theme.cardPlaceholderText, theme.cardPlaceholder) >= 4.5);
            QVERIFY(contrastRatio(theme.badgeInfoText, theme.badgeInfo) >= 4.5);
            QVERIFY(contrastRatio(theme.viewerTextDim, theme.viewerSurround) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.panel) >= 4.5);
            QVERIFY(contrastRatio(theme.statusText, theme.statusBar) >= 4.5);
            // Placeholders carry guidance (example searches, what to ask), so
            // they read as text.
            QVERIFY(contrastRatio(theme.placeholderText, theme.field) >= 4.5);
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::PlaceholderText), theme.placeholderText);
            // The Source monitor's marked range is information: 3:1 on its panel.
            QVERIFY(contrastRatio(theme.inOutRange, theme.panel) >= 3.0);
            // The changed-on-disk triangle reads on its bar.
            QVERIFY(contrastRatio(theme.warningGlyph, theme.warningSurface) >= 3.0);
            // A clip the agent adds is named in its purple, on its wash over the lane.
            QVERIFY(contrastRatio(theme.assistantText, over(theme.trackLane, theme.proposalAddFill)) >= 4.5);
            // A button focused from the keyboard is ringed in the accent at 35%.
            QCOMPARE(theme.focusHalo.rgb(), theme.accent.rgb());
            QCOMPARE(theme.focusHalo.alpha(), 89);
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
            // The conversation is open beside the picture, so it, not the
            // timeline's bar, carries the proposal.
            QVERIFY(window.m_workspace->assistantOpen());
            QVERIFY(!window.m_aiBar->isVisible());
            QCoreApplication::processEvents();
            const QImage image = window.grab().toImage();
            // The bars are the theme's: toolbar, status bar, and dock title strips.
            auto *toolbar = window.findChild<QToolBar *>();
            QVERIFY(toolbar);
            QCOMPARE(dominantColor(toolbar->grab().toImage()), theme.chrome.rgb());
            QCOMPARE(dominantColor(window.statusBar()->grab().toImage()), theme.statusBar.rgb());
            // The window is titled with the project, so the Window menu,
            // Mission Control and the Dock name it, and the toolbar shows the same.
            QCOMPARE(window.windowTitle(), QStringLiteral("Harbor Teaser"));
            QCOMPARE(window.m_projectTitle->text(), window.windowTitle());
            // The title is 13px semibold, as the launcher's and macOS's, and
            // centred on the window (the toolbar spans it), clear of the
            // groups either side.
            QLabel *title = window.m_projectTitle;
            QCOMPARE(title->font().pixelSize(), 13);
            QVERIFY2(std::abs(title->geometry().center().x() - toolbar->width() / 2) <= 1,
                     qPrintable(QStringLiteral("%1 of %2").arg(title->geometry().center().x()).arg(toolbar->width())));
            QVERIFY(title->geometry().left() > window.m_toolbarLeftEnd->geometry().right());
            QVERIFY(title->geometry().right() < window.m_quality->geometry().left());
            // The controls at the right stand 8px apart, bezel to bezel, and
            // Render has the pop-ups' bezel, not bare text.
            auto *exportButton = toolbar->findChild<QToolButton *>(QStringLiteral("toolbarPrimary"));
            auto *render = toolbar->findChild<QToolButton *>(QStringLiteral("toolbarQuiet"));
            QVERIFY(exportButton && render);
            const QList<QWidget *> controls{window.m_quality, window.m_canvasPreset, exportButton, render};
            for (int i = 1; i < controls.size(); ++i)
                QCOMPARE(controls.at(i)->x() - controls.at(i - 1)->geometry().right() - 1, 8);
            QCOMPARE(render->height(), window.m_quality->height());
            QCOMPARE(dominantColor(render->grab().toImage()), (render->isEnabled() ? theme.button : theme.buttonDisabled).rgb());
            // Kerf's conversation is open, so its switch at the toolbar's end,
            // a group of its own, is chosen: the accent wash and edge, its
            // bubble the accent, as a chosen mode in a title strip.
            QToolButton *chat = window.m_chatToggle;
            QVERIFY(chat->isVisible() && chat->isChecked());
            QVERIFY(chat->x() - render->geometry().right() - 1 > 8);
            QCOMPARE(chat->height(), render->height());
            {
                // The bezel's fill, clear of its edge and its symbol: at rest,
                // under the pointer, and held.
                const auto fill = [chat] {
                    const QImage image = chat->grab().toImage();
                    return image.pixelColor(QPoint(3, chat->height() / 2) * image.devicePixelRatio()).rgb();
                };
                QCOMPARE(fill(), theme.accentTint.rgb());
                chat->setAttribute(Qt::WA_UnderMouse, true);
                QCOMPARE(fill(), theme.accentTint.rgb());
                chat->setAttribute(Qt::WA_UnderMouse, false);
                chat->setDown(true);
                QCOMPARE(fill(), theme.accentTintPressed.rgb());
                chat->setDown(false);
                QCOMPARE(ink(chat->icon(), QIcon::Normal, QIcon::On), theme.accentText.rgb());
                QCOMPARE(ink(chat->icon(), QIcon::Normal, QIcon::Off), theme.glyph.rgb());
            }
#if defined(Q_OS_MACOS) && QT_VERSION >= QT_VERSION_CHECK(6, 9, 0)
            // The toolbar is the title bar: the window runs up under a
            // transparent one, and its commands start past the window buttons.
            QVERIFY(window.windowFlags().testFlag(Qt::ExpandedClientAreaHint));
            QVERIFY(window.windowFlags().testFlag(Qt::NoTitleBarBackgroundHint));
            QCOMPARE(toolbar->geometry().top(), 0);
            QVERIFY(window.m_windowButtons->isVisible());
            QVERIFY(toolbar->mapTo(&window, window.m_windowButtons->geometry().topRight()).x() >= 72);
            for (QAction *action : toolbar->actions())
                if (QWidget *item = toolbar->widgetForAction(action); item && item->isVisible() && item != window.m_windowButtons)
                    QVERIFY(item->x() > window.m_windowButtons->geometry().right());
#endif
            // Nothing is ringed: the window handed its first control focus as
            // it opened, which is no keyboard navigation.
            QVERIFY(!window.findChild<QWidget *>(QStringLiteral("focusHalo")));
            // Fields draw their placeholders in the palette's colour, which
            // reads at 4.5:1, not in their text colour at half strength (as
            // Qt's style sheets draw them unless told).
            for (QWidget *field : std::initializer_list<QWidget *>{window.m_effects->m_search, window.m_chatInput, window.m_cloudToken}) {
                field->ensurePolished();
                for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive})
                    QCOMPARE(field->palette().color(group, QPalette::PlaceholderText), theme.placeholderText);
            }
            // Timecodes in the accent sit on the panel accentText is made to
            // read on: the Program monitor's, and the Source monitor's, a tab
            // behind it.
            QList<QLabel *> times = window.findChildren<QLabel *>(QStringLiteral("transportTime"));
            for (QLabel *time : window.m_sourceMonitor->findChildren<QLabel *>(QStringLiteral("transportTime")))
                if (!times.contains(time)) times.append(time);
            QCOMPARE(times.size(), 2);
            for (QLabel *time : times) {
                QCOMPARE(time->palette().color(QPalette::WindowText).rgb(), theme.accentText.rgb());
                QCOMPARE(dominantColor(time->parentWidget()->grab().toImage()), theme.panel.rgb());
            }
            // The launcher's chrome: #f5f5f5 in light, #2c2c2e in dark.
            QVERIFY(dark ? qGray(dominantColor(toolbar->grab().toImage())) < 60 : qGray(dominantColor(toolbar->grab().toImage())) > 220);
            // Tabs show no close button until the pointer is over them; then
            // it reads on the tab.
            const auto closeButtons = window.findChildren<QWidget *>(QStringLiteral("tabCloseButton"));
            QVERIFY(closeButtons.size() >= 3);
            for (const QWidget *close : closeButtons) QVERIFY(!close->isVisible());
            QWidget *programClose = nullptr;
            for (QWidget *close : closeButtons)
                if (close->parentWidget() == window.m_workspace->panel(QStringLiteral("program"))->tabWidget()) programClose = close;
            QVERIFY(programClose);
            QEvent enter(QEvent::Enter);
            QCoreApplication::sendEvent(programClose->parentWidget(), &enter);
            QVERIFY(programClose->isVisible());
            {
                const QImage tab = programClose->parentWidget()->grab().toImage();
                const qreal ratio = tab.devicePixelRatio();
                const QRect area(programClose->geometry().topLeft() * ratio, programClose->size() * ratio);
                QVERIFY(highestContrast(tab, area, theme.panelTitleActive) >= 4.5);
            }
            QEvent leave(QEvent::Leave);
            QCoreApplication::sendEvent(programClose->parentWidget(), &leave);
            QVERIFY(!programClose->isVisible());
            // The chosen tool is a quiet chip with its symbol in the accent.
            const QImage tool = window.m_toolPalette->widgetForAction(window.m_commands->action(QStringLiteral("tool.selection")))->grab().toImage();
            const QColor toolFill = tool.pixelColor(qRound(3 * tool.devicePixelRatio()), tool.height() / 2);
            QCOMPARE(toolFill.rgb(), theme.toolSelected.rgb());
            QVERIFY(highestContrast(tool, tool.rect(), toolFill) >= 3.0);
            // An unavailable command's symbol is dimmed in both appearances,
            // never brightened, as Qt's own disabled pixmap does on dark.
            for (const char *id : {"edit.undo", "edit.redo"}) {
                QAction *action = window.m_commands->action(QString::fromLatin1(id));
                QVERIFY(!action->isEnabled());
                const QImage dim = action->icon().pixmap(QSize(20, 20), 2.0, QIcon::Disabled).toImage();
                QColor ink(0, 0, 0, 0);
                for (int y = 0; y < dim.height(); ++y)
                    for (int x = 0; x < dim.width(); ++x)
                        if (dim.pixelColor(x, y).alpha() > ink.alpha()) ink = dim.pixelColor(x, y);
                QCOMPARE(ink.rgb(), theme.glyphDisabled.rgb());
            }
            // A title strip's details are plain text on the strip, not a dead button.
            for (const char *id : {"viewerDetail", "timelineDetail"}) {
                auto *detail = window.findChild<QToolButton *>(QString::fromLatin1(id));
                QVERIFY(detail && detail->isVisible() && !detail->isEnabled());
                QCOMPARE(dominantColor(detail->parentWidget()->grab(detail->geometry()).toImage()), theme.panelTitle.rgb());
            }
            save(image, QStringLiteral("editor-%1").arg(name));
            // With the conversation closed, the timeline's bar stands in for
            // it: the window's chrome with a purple stripe, not amber, and
            // the launcher's 22px buttons.
            window.m_workspace->setAssistantOpen(false);
            QCoreApplication::processEvents();
            QVERIFY(window.m_aiBar->isVisible());
            {
                const QImage aiBar = window.m_aiBar->grab().toImage();
                QCOMPARE(dominantColor(aiBar), theme.chrome.rgb());
                QCOMPARE(aiBar.pixelColor(QPoint(1, aiBar.height() / 2) * aiBar.devicePixelRatio()), theme.assistant);
                QCOMPARE(amberPixels(aiBar), 0);
                for (QWidget *control : window.m_aiBar->findChildren<QToolButton *>())
                    if (control->isVisible()) QCOMPARE(control->height(), 22);
            }
            window.m_workspace->setAssistantOpen(true);
            QCoreApplication::processEvents();
            QVERIFY(!window.m_aiBar->isVisible());

            // The toolbar's pop-up opens as macOS's menu: rounded, clipped to
            // its corners, a 22px row per choice, and the row under the
            // pointer (here the current choice) filled with the accent.
            QVERIFY(dynamic_cast<PopupItemDelegate *>(window.m_quality->itemDelegate()));
            window.m_quality->showPopup();
            QWidget *list = window.m_quality->view()->window();
            QVERIFY(QTest::qWaitForWindowExposed(list));
            QVERIFY(list->testAttribute(Qt::WA_TranslucentBackground));
            {
                const QImage popup = list->grab().toImage();
                save(popup, QStringLiteral("popup-%1").arg(name));
                const qreal ratio = popup.devicePixelRatio();
                QCOMPARE(popup.pixelColor(0, 0).alpha(), 0);
                const QRect row = window.m_quality->view()->visualRect(window.m_quality->model()->index(0, 0));
                QCOMPARE(row.height(), 22);
                const QPoint inside = window.m_quality->view()->viewport()->mapTo(list, QPoint(row.left() + 2, row.center().y()));
                QCOMPARE(popup.pixelColor(inside * ratio).rgb(), theme.menuSelection.rgb());
                QCOMPARE(amberPixels(popup), 0);
            }
            window.m_quality->hidePopup();
            for (QComboBox *picker : {window.m_quality, window.m_canvasPreset}) QVERIFY(popupShowsEveryRow(picker));
            // So does a clip's context menu: rounded and clipped to its
            // corners, the row under the pointer a rounded accent fill.
            {
                QImage shape;
                QRect row;
                QList<QRect> others;
                bool translucent = false;
                bool symbols = false;
                bool iconColumn = false;
                // Captured while it is open, and closed whatever happens.
                QTimer::singleShot(0, &window, [&] {
                    auto *menu = qobject_cast<QMenu *>(QApplication::activePopupWidget());
                    if (!menu) return;
                    const auto close = qScopeGuard([menu] { menu->close(); });
                    for (QAction *action : menu->actions()) {
                        symbols = symbols || !action->icon().isNull();
                        if (!action->isEnabled() || action->isSeparator()) continue;
                        if (row.isValid()) {
                            others << menu->actionGeometry(action);
                            continue;
                        }
                        menu->setActiveAction(action);
                        row = menu->actionGeometry(action);
                    }
                    translucent = menu->testAttribute(Qt::WA_TranslucentBackground);
                    iconColumn = menu->property("iconColumn").toBool();
                    shape = menu->grab().toImage();
                });
                QTimer::singleShot(2000, &window, [] {
                    if (QWidget *popup = QApplication::activePopupWidget()) popup->close();
                });
                emit window.m_timeline->clipMenuRequested(QStringLiteral("v1b"), window.mapToGlobal(QPoint(400, 300)));
                QVERIFY(!shape.isNull() && row.isValid() && !others.isEmpty());
                save(shape, QStringLiteral("menu-%1").arg(name));
                QVERIFY(translucent);
                const qreal ratio = shape.devicePixelRatio();
                QCOMPARE(shape.pixelColor(0, 0).alpha(), 0);
                QCOMPARE(shape.pixelColor(QPoint(row.left() + 2, row.center().y()) * ratio).rgb(), theme.menuSelection.rgb());
                QCOMPARE(amberPixels(shape), 0);
                // Menus show no symbols, as macOS's, though Split and Delete
                // have them for the toolbar: each row's first 20px are its
                // fill, the highlighted row's too, and its text starts 22px
                // in, as in a menu of commands without symbols.
                QVERIFY(QCoreApplication::testAttribute(Qt::AA_DontShowIconsInMenus));
                QVERIFY(symbols && iconColumn);
                others.prepend(row);
                for (const QRect &item : std::as_const(others)) {
                    const QColor fill = item == row ? theme.menuSelection : theme.menu;
                    for (int y = item.center().y() - 4; y <= item.center().y() + 4; ++y)
                        for (int x = item.left() + 1; x <= item.left() + 20; ++x)
                            QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), fill.rgb());
                    if (item != row)
                        QVERIFY(highestContrast(shape, QRect(QPoint(item.left() + 21, item.top()) * ratio, QSize(5, item.height()) * ratio),
                                                fill) >= 3.0);
                }
                // Nor do the menu bar's menus, native on macOS.
                for (QAction *menuAction : window.menuBar()->actions())
                    if (QMenu *menu = menuAction->menu())
                        for (QAction *action : menu->actions()) QVERIFY2(!action->isIconVisibleInMenu(), qPrintable(action->text()));
            }
            // A menu with checks (the Window menu's panels) pads its rows by
            // less, so its text starts where a menu's without checks does.
            {
                QMenu *panels = nullptr;
                for (QAction *action : window.menuBar()->actions())
                    if (action->text() == QStringLiteral("Window")) panels = action->menu();
                QVERIFY(panels);
                panels->popup(window.mapToGlobal(QPoint(400, 300)));
                QVERIFY(QTest::qWaitForWindowExposed(panels));
                QVERIFY(panels->property("checkColumn").toBool());
                const QImage shape = panels->grab().toImage();
                save(shape, QStringLiteral("menu-checks-%1").arg(name));
                const qreal ratio = shape.devicePixelRatio();
                QCOMPARE(shape.pixelColor(0, 0).alpha(), 0);
                QCOMPARE(amberPixels(shape), 0);
                // Its checks are the pop-ups' thin check, 7px into the row,
                // not Fusion's tick at the row's start.
                QAction *checked = nullptr;
                QAction *submenu = nullptr;
                for (QAction *action : panels->actions()) {
                    if (!checked && action->isChecked()) checked = action;
                    if (!submenu && action->menu()) submenu = action;
                }
                QVERIFY(checked && submenu);
                const QRect check = panels->actionGeometry(checked);
                for (int y = check.center().y() - 4; y <= check.center().y() + 4; ++y)
                    for (int x = check.left(); x <= check.left() + 4; ++x)
                        QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), theme.menu.rgb());
                QVERIFY(highestContrast(shape, QRect(QPoint(check.left() + 6, check.center().y() - 5) * ratio, QSize(11, 11) * ratio),
                                        theme.menu) >= 3.0);
                // A submenu's chevron is thin and open, not Fusion's filled
                // triangle: inside its mouth is the row's fill.
                const QRect arrow = panels->actionGeometry(submenu);
                for (int y = arrow.center().y() - 1; y <= arrow.center().y() + 1; ++y)
                    for (int x = arrow.right() - 13; x <= arrow.right() - 12; ++x)
                        QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), theme.menu.rgb());
                QVERIFY(highestContrast(shape, QRect(QPoint(arrow.right() - 14, arrow.center().y() - 5) * ratio, QSize(7, 11) * ratio),
                                        theme.menu) >= 3.0);
                panels->close();
                QVERIFY(!window.binMenu(QString())->property("checkColumn").toBool());
            }

            // Settings.
            window.m_preferences->show();
            QVERIFY(QTest::qWaitForWindowExposed(window.m_preferences));
            QCoreApplication::processEvents();
            const QImage settings = window.m_preferences->grab().toImage();
            QCOMPARE(settings.pixelColor(QPoint(4, settings.height() - 4)).rgb(), theme.dialog.rgb());
            // Sized to what it shows, with no Close button under empty space.
            QVERIFY(window.m_preferences->height() < 400);
            QVERIFY(!window.m_preferences->findChild<QDialogButtonBox *>());
            save(settings, QStringLiteral("settings-%1").arg(name));
            // Advanced opens below, and the window grows to show it.
            auto *advanced = window.m_preferences->findChild<QToolButton *>(QStringLiteral("disclosure"));
            QVERIFY(advanced);
            // Its chevron, the only sign of whether it is open, reads on the
            // dialog (3:1), closed and open.
            QCOMPARE(ink(advanced->icon()), theme.textSecondary.rgb());
            const int closedHeight = window.m_preferences->height();
            advanced->click();
            QVERIFY(window.m_advancedSettings->isVisible());
            QCOMPARE(ink(advanced->icon()), theme.textSecondary.rgb());
            QVERIFY(window.m_preferences->height() > closedHeight);
            advanced->click();
            window.m_preferences->hide();

            // A representative dialog: the preset's media bindings.
            QTimer::singleShot(0, &window, [&] {
                auto *dialog = window.findChild<QDialog *>(QStringLiteral("presetBindingsDialog"));
                QVERIFY(dialog);
                QTimer::singleShot(2000, dialog, &QDialog::reject);
                for (const char *slot : {"presetMedia_Opening", "presetMedia_Logo"})
                    if (auto *choice = dialog->findChild<QComboBox *>(QString::fromLatin1(slot))) choice->setCurrentIndex(1);
                QCoreApplication::processEvents();
                const QImage bindings = dialog->grab().toImage();
                QCOMPARE(dominantColor(bindings), theme.dialog.rgb());
                // The default button is the accent, as in macOS dialogs.
                QPushButton *create = dialog->findChild<QDialogButtonBox *>()->button(QDialogButtonBox::Ok);
                QCOMPARE(create->text(), QStringLiteral("Create Proposal"));
                // Its row of pop-up and seconds field is one 22px line, as in the launcher.
                auto *choice = dialog->findChild<QComboBox *>(QStringLiteral("presetMedia_Opening"));
                auto *from = dialog->findChild<QDoubleSpinBox *>(QStringLiteral("presetFrom_Opening"));
                QVERIFY(choice && from);
                QCOMPARE(choice->height(), 22);
                QCOMPARE(from->height(), 22);
                QCOMPARE(create->height(), 22);
                QCOMPARE(from->geometry().center().y(), choice->geometry().center().y());
                QVERIFY(popupShowsEveryRow(choice));
                // Its pop-ups are buttons' bezels, as the launcher's Frame
                // Rate and the toolbar's pickers, not fields like Text 1.
                QVERIFY(popUpHasButtonBezel(bindings, choice, theme.button));
                QVERIFY(create->isEnabled() && create->isDefault());
                const QImage createImage = create->grab().toImage();
                const QColor createFill = createImage.pixelColor(QPoint(6, create->height() / 2) * createImage.devicePixelRatio());
                QVERIFY2(std::abs(createFill.hue() - theme.accent.hue()) <= 4, qPrintable(createFill.name()));
                // Sized to its slots, as macOS dialogs are, with no empty
                // window under them; Details grows it, and closing shrinks it back.
                QVERIFY2(dialog->height() < 400, qPrintable(QString::number(dialog->height())));
                save(bindings, QStringLiteral("dialog-%1").arg(name));
                auto *details = dialog->findChild<QToolButton *>(QStringLiteral("disclosure"));
                QVERIFY(details);
                QCOMPARE(ink(details->icon()), theme.textSecondary.rgb());
                const int closedHeight = dialog->height();
                details->click();
                QCOMPARE(ink(details->icon()), theme.textSecondary.rgb());
                QVERIFY(dialog->height() > closedHeight);
                details->click();
                QCOMPARE(dialog->height(), closedHeight);
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
            QVERIFY(!window.m_aiBar->isVisible());
            // The proposal's default button, Accept, is the accent.
            QPushButton *primary = window.m_acceptButton;
            QVERIFY(primary->isVisible() && primary->isEnabled());
            QCOMPARE(primary->objectName(), QStringLiteral("primaryButton"));
            const QImage primaryImage = primary->grab().toImage();
            const QColor fill = primaryImage.pixelColor(QPoint(6, primary->height() / 2) * primaryImage.devicePixelRatio());
            QVERIFY2(std::abs(fill.hue() - theme.accent.hue()) <= 4 && std::abs(fill.value() - theme.accent.value()) <= 24,
                     qPrintable(fill.name()));
            const QImage states = window.grab().toImage();
            // The proposal's diff: additions green, removals red, the rest secondary.
            int added = 0;
            int removed = 0;
            for (QTextBlock block = window.m_proposalDiff->document()->begin(); block.isValid(); block = block.next()) {
                const QString line = block.text();
                if (line.isEmpty()) continue;
                const bool header = line.startsWith(QStringLiteral("+++")) || line.startsWith(QStringLiteral("---"));
                QColor expected = theme.textSecondary;
                if (!header && line.startsWith(QLatin1Char('+'))) {
                    expected = theme.success;
                    ++added;
                } else if (!header && line.startsWith(QLatin1Char('-'))) {
                    expected = theme.error;
                    ++removed;
                }
                QCOMPARE(block.begin().fragment().charFormat().foreground().color(), expected);
            }
            QVERIFY(added > 0 && removed > 0);
            // Reject, then the default Accept, at the right.
            QVERIFY(window.m_rejectButton->geometry().right() < window.m_acceptButton->geometry().left());
            QCOMPARE(window.m_acceptButton->height(), 22);
            // So does every other row of buttons, as macOS orders them: the
            // timeline's bar (shown with the conversation closed) ends in
            // Accept and the banner in Load from Disk, each 8px past the
            // button before it.
            window.m_workspace->setAssistantOpen(false);
            QCoreApplication::processEvents();
            {
                QList<QWidget *> bar;
                for (QToolButton *button : window.m_aiBar->findChildren<QToolButton *>())
                    if (button->isVisible()) bar.append(button);
                std::sort(bar.begin(), bar.end(), [](const QWidget *a, const QWidget *b) { return a->x() < b->x(); });
                QCOMPARE(bar.size(), 3);
                QCOMPARE(bar.constLast()->objectName(), QStringLiteral("aiAccept"));
                for (int i = 1; i < bar.size(); ++i) QCOMPARE(bar.at(i)->x() - bar.at(i - 1)->geometry().right() - 1, 8);
                QCOMPARE(window.m_loadFromDisk->x() - window.m_keepEditorVersion->geometry().right() - 1, 8);
                // Their trailing buttons line up with the toolbar's last
                // control, the conversation's switch, 8px in from the
                // window's edge.
                for (QWidget *trailing : std::initializer_list<QWidget *>{window.m_chatToggle, window.m_loadFromDisk, bar.constLast()})
                    QCOMPARE(trailing->mapTo(&window, QPoint(trailing->width(), 0)).x(), window.width() - 8);
            }
            // Kerf's suggestions are purple pills sized to their labels.
            for (QWidget *row : {window.m_nextSteps, window.m_chatNextSteps}) {
                const auto chips = row->findChildren<QPushButton *>(QStringLiteral("nextStep"));
                QVERIFY(!chips.isEmpty());
                for (QPushButton *chip : chips) {
                    QCOMPARE(chip->height(), 22);
                    QCOMPARE(dominantColor(chip->grab().toImage()), theme.assistantSurface.rgb());
                    QVERIFY(chip->width() < 160);
                }
            }
            window.m_workspace->setAssistantOpen(true);
            QCoreApplication::processEvents();
            // The bar and its text are neutral; only the 14-point triangle is orange.
            for (QWidget *part : window.m_divergedBanner->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
                if (part != window.m_divergedGlyph) QCOMPARE(amberPixels(part->grab().toImage()), 0);
            QCOMPARE(dominantColor(window.m_divergedBanner->grab().toImage()), theme.warningSurface.rgb());
            // The banner is its own bar, not a second row of the toolbar: in
            // dark a step lighter than it, in light a warm tint.
            QVERIFY(theme.warningSurface.rgb() != theme.chrome.rgb());
            if (dark) QVERIFY(theme.warningSurface.lightness() > theme.chrome.lightness());
            // The assistant's view, scrolled, has an edge under its header.
            QVERIFY(window.m_chatScroll->verticalScrollBar()->value() > 0);
            auto *scrollEdge = window.findChild<QFrame *>(QStringLiteral("scrollEdge"));
            QVERIFY(scrollEdge && scrollEdge->isVisible());
            QCOMPARE(scrollEdge->grab().toImage().pixelColor(4, 0).rgb(), theme.divider.rgb());
            save(states, QStringLiteral("states-%1").arg(name));

            // Where the timeline's scroll bars meet is the panel, not a framed
            // square. Both bars are shown here, whatever the fonts' heights.
            {
                QScrollArea *scroll = window.m_timelineScroll;
                const Qt::ScrollBarPolicy verticalPolicy = scroll->verticalScrollBarPolicy();
                const Qt::ScrollBarPolicy horizontalPolicy = scroll->horizontalScrollBarPolicy();
                scroll->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOn);
                scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOn);
                QCoreApplication::processEvents();
                const QScrollBar *vertical = scroll->verticalScrollBar();
                const QScrollBar *horizontal = scroll->horizontalScrollBar();
                QVERIFY(vertical->isVisible() && horizontal->isVisible());
                const QRect corner(QPoint(vertical->mapTo(&window, QPoint()).x(), horizontal->mapTo(&window, QPoint()).y()),
                                   QSize(vertical->width(), horizontal->height()));
                const QImage cornerImage = window.grab(corner).toImage();
                for (int y = 0; y < cornerImage.height(); ++y)
                    for (int x = 0; x < cornerImage.width(); ++x) QCOMPARE(cornerImage.pixelColor(x, y).rgb(), theme.panel.rgb());
                scroll->setVerticalScrollBarPolicy(verticalPolicy);
                scroll->setHorizontalScrollBarPolicy(horizontalPolicy);
            }

            // The inspector: every field starts at one edge, the clip's
            // timing and an effect's parameters alike; number fields are as
            // wide as their values need; a colour is a well; a file is a
            // field and a Choose… button (a look, as FilmCraft's LUT
            // parameters, stands in for one).
            window.m_effectControls->m_effectDefs.insert(QStringLiteral("test.look"), QJsonObject{
                {"id", "test.look"}, {"name", "Look"}, {"params", QJsonArray{QJsonObject{
                    {"name", "look_lut"}, {"type", "file"}, {"doc", "Look LUT"}, {"extensions", QJsonArray{"cube", "3dl"}}, {"default", ""}}}}});
            window.applySnapshot(QJsonObject{{"revision", 14}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "wide"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"effects", QJsonArray{QJsonObject{{"fx", "light.edgeglow"}, {"enabled", false}, {"params", QJsonObject{}}},
                                                       QJsonObject{{"fx", "test.look"}, {"enabled", true},
                                                                   {"params", QJsonObject{{"look_lut", "looks/harbor-dusk.cube"}}}}}}}}}}}}}}});
            window.selectClip(QStringLiteral("c1"));
            window.m_workspace->panel(QStringLiteral("effectControls"))->toggleView(true);
            window.m_workspace->panel(QStringLiteral("effectControls"))->setAsCurrentTab();
            QCoreApplication::processEvents();
            EffectControlsPanel *inspector = window.m_effectControls;
            QVERIFY(inspector->isVisible());
            const int fieldEdge = inspector->m_position->mapTo(inspector, QPoint()).x();
            QList<QWidget *> fields{inspector->m_duration, inspector->m_in, inspector->m_out, inspector->m_speed};
            for (QWidget *field : inspector->findChildren<QWidget *>())
                if (field->objectName().startsWith(QStringLiteral("effectParam_"))) fields << field;
            QVERIFY(fields.size() > 6);
            // The file's row is a field's height, with its Choose… button
            // in the push buttons' bezel.
            auto *lookFile = inspector->findChild<QWidget *>(QStringLiteral("effectParam_look_lut"));
            QVERIFY(lookFile && lookFile->findChild<QPushButton *>());
            QCOMPARE(lookFile->height(), 22);
            QCOMPARE(lookFile->findChild<QLineEdit *>()->text(), QStringLiteral("looks/harbor-dusk.cube"));
            for (QWidget *field : fields) {
                QCOMPARE(field->mapTo(inspector, QPoint()).x(), fieldEdge);
                if (qobject_cast<QDoubleSpinBox *>(field)) QVERIFY(field->width() <= 96);
                if (auto *well = qobject_cast<QPushButton *>(field)) {
                    QCOMPARE(well->size(), QSize(44, 22));
                    QVERIFY(well->text().isEmpty());
                }
            }
            // The clip's name starts on the labels' edge, and every form keeps
            // one rhythm, 6px between rows.
            auto *timing = qobject_cast<QFormLayout *>(inspector->m_position->parentWidget()->layout());
            QVERIFY(timing);
            auto *positionLabel = qobject_cast<QLabel *>(timing->labelForField(inspector->m_position));
            QVERIFY(positionLabel);
            QCOMPARE(inspector->m_clipName->indent(), 0);
            QCOMPARE(inspector->m_clipName->mapTo(inspector, inspector->m_clipName->contentsRect().topLeft()).x(),
                     positionLabel->mapTo(inspector, positionLabel->contentsRect().topLeft()).x());
            for (const QFormLayout *form : inspector->findChildren<QFormLayout *>()) QCOMPARE(form->verticalSpacing(), 6);
            // A check box row is as tall as a field's, so the rows keep a
            // 28px pitch to the last.
            int checkRows = 0;
            for (QCheckBox *check : inspector->findChildren<QCheckBox *>())
                if (check->objectName().startsWith(QStringLiteral("effectParam_"))) {
                    QCOMPARE(check->height(), 22);
                    ++checkRows;
                }
            QCOMPARE(checkRows, 1);
            // Each effect is headed by its name in semibold, 18px below the
            // row before it, the second as the first; a hairline 8px under
            // the first effect's last row sets the second apart. A clip with
            // no properties shows no empty form above them.
            QVERIFY(inspector->m_propertyForm->parentWidget()->isHidden());
            {
                const auto effectBoxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
                QCOMPARE(effectBoxes.size(), 2);
                const auto top = [inspector](const QWidget *widget) { return widget->mapTo(inspector, QPoint()).y(); };
                const auto bottom = [&top](const QWidget *widget) { return top(widget) + widget->height(); };
                const auto header = [&top](const QWidget *box) {
                    return top(box->findChild<QToolButton *>(QStringLiteral("effectRemove")));
                };
                for (const QWidget *box : effectBoxes)
                    QCOMPARE(box->findChild<QCheckBox *>(QStringLiteral("effectEnabled"))->font().weight(), QFont::DemiBold);
                QCOMPARE(header(effectBoxes.at(0)) - bottom(inspector->m_speed), 18);
                QCOMPARE(header(effectBoxes.at(1)) - bottom(effectBoxes.at(0)), 18);
                QCOMPARE(top(effectBoxes.at(1)) - bottom(effectBoxes.at(0)), 8);
            }
            // The picture shows the whole form: the window grows until the
            // panel needs no scrolling.
            QScrollBar *inspectorScroll = nullptr;
            for (QScrollArea *area : inspector->findChildren<QScrollArea *>())
                if (area->isVisible()) inspectorScroll = area->verticalScrollBar();
            for (int grow = 0; inspectorScroll && inspectorScroll->maximum() > 0 && grow < 10; ++grow) {
                window.resize(window.width(), window.height() + 20);
                QCoreApplication::processEvents();
            }
            const QImage inspectorImage = window.m_workspace->panel(QStringLiteral("effectControls"))->grab().toImage();
            // The hairline runs across the column, on the panel.
            {
                const QWidget *second = inspector->findChildren<QWidget *>(QStringLiteral("effectBox")).at(1);
                const QPoint rule = second->mapTo(window.m_workspace->panel(QStringLiteral("effectControls")),
                                                  QPoint(second->width() / 2, 0));
                const qreal ratio = inspectorImage.devicePixelRatio();
                QCOMPARE(inspectorImage.pixelColor(rule * ratio).rgb(), theme.divider.rgb());
                QCOMPARE(inspectorImage.pixelColor((rule - QPoint(0, 1)) * ratio).rgb(), theme.panel.rgb());
                QCOMPARE(inspectorImage.pixelColor((rule + QPoint(0, 1)) * ratio).rgb(), theme.panel.rgb());
            }
            save(inspectorImage, QStringLiteral("inspector-%1").arg(name));
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
        QVERIFY(!window.m_disclosures.isEmpty());
        for (QToolButton *disclosure : std::as_const(window.m_disclosures))
            QCOMPARE(ink(disclosure->icon()), editorTheme().textSecondary.rgb());
        // Re-applying the same appearance changes nothing.
        applyEditorAppearance();
        QCOMPARE(changed.count(), before + 1);
    }

    // The whole editor in the Editing workspace with Kerf's conversation
    // open beside the picture, as a Short shows with its comments open: a
    // 16:9, a 9:16 and a 1:1 project, in both appearances. The picture is as
    // large as the viewer allows, in the project's shape, and the
    // conversation's column sits directly on the viewer's right, the full
    // height of the row. With KERF_SCREENSHOT_DIR set, each is saved as
    // editor-<format>-<scheme>.png, and the 9:16 one with the conversation
    // closed as editor-9x16-closed-<scheme>.png.
    void editorShowsEveryFormatBesideItsConversation() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        // A harbor at dawn in the project's shape: a sky over a sea of one colour.
        const QColor sea("#24465a");
        const auto picture = [&sea](const QSize &size) {
            QImage image(size, QImage::Format_RGB32);
            QPainter painter(&image);
            QLinearGradient sky(0, 0, 0, size.height() * 0.55);
            sky.setColorAt(0, QColor("#f2c79b"));
            sky.setColorAt(1, QColor("#8fb1d3"));
            painter.fillRect(QRect(0, 0, size.width(), size.height() * 55 / 100), sky);
            painter.fillRect(QRect(0, size.height() * 55 / 100, size.width(), size.height()), sea);
            painter.setPen(Qt::NoPen);
            painter.setBrush(QColor("#fff1dc"));
            painter.drawEllipse(QPointF(size.width() * 0.7, size.height() * 0.3), size.width() * 0.06, size.width() * 0.06);
            return image;
        };
        struct Format {
            const char *name;
            QSize canvas;
        };
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            for (const Format &format : {Format{"16x9", QSize(1920, 1080)}, Format{"9x16", QSize(1080, 1920)}, Format{"1x1", QSize(1080, 1080)}}) {
                QSettings().remove(QStringLiteral("workspace"));
                QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
                QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
                timeline.insert(QStringLiteral("width"), format.canvas.width());
                timeline.insert(QStringLiteral("height"), format.canvas.height());
                snapshot.insert(QStringLiteral("timeline"), timeline);
                snapshot.remove(QStringLiteral("pendingProposal"));
                snapshot.remove(QStringLiteral("proposalTimeline"));
                MainWindow window{QString(), QString()};
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshot);
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                window.m_viewer->setFrame(picture(format.canvas / 4), 0.0);
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
                window.appendConversation(QStringLiteral("Kerf"),
                    QStringLiteral("I trimmed the close-up by 1.5 seconds, removed the drone pass, and added a lower third at 12 s."));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b · removed v1c"));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Now caption the dialogue."));
                window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                                QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music under the dialogue"}},
                                                QJsonObject{{"label", "Match the colour"}, {"request", "Match the close-up to the wide"}}});
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                QCoreApplication::processEvents();
                const QString label = QString::fromLatin1(format.name);
                QCOMPARE(window.m_canvasSize, format.canvas);

                ads::CDockAreaWidget *viewerArea = window.m_workspace->panel(QStringLiteral("program"))->dockAreaWidget();
                ads::CDockAreaWidget *chatArea = window.m_workspace->panel(QStringLiteral("assistant"))->dockAreaWidget();
                QVERIFY(window.m_viewer->isVisible() && chatArea->isVisible());
                QVERIFY(!window.m_sourceMonitor->isVisible());
                const QRect viewerRect(viewerArea->mapTo(&window, QPoint()), viewerArea->size());
                const QRect chatRect(chatArea->mapTo(&window, QPoint()), chatArea->size());
                QVERIFY2(chatRect.left() - viewerRect.right() <= 3, qPrintable(label));
                QCOMPARE(chatRect.top(), viewerRect.top());
                QCOMPARE(chatRect.bottom(), viewerRect.bottom());
                QCOMPARE(chatRect.right(), window.width() - 1);
                // The picture keeps the project's shape and fills the viewer
                // one way or the other, inside its 8px margin.
                const QRectF video = window.m_viewer->videoRect();
                QVERIFY2(std::abs(video.width() / video.height() - double(format.canvas.width()) / format.canvas.height()) < 0.01,
                         qPrintable(label));
                const QRectF room = QRectF(window.m_viewer->rect()).adjusted(8, 8, -8, -8);
                QVERIFY2(std::abs(video.height() - room.height()) < 1 || std::abs(video.width() - room.width()) < 1, qPrintable(label));
                QVERIFY(room.contains(video.adjusted(0.5, 0.5, -0.5, -0.5)));
                const QImage image = window.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const auto at = [&](const QPointF &viewerPoint) {
                    return image.pixelColor((QPointF(window.m_viewer->mapTo(&window, QPoint())) + viewerPoint).toPoint() * ratio).rgb();
                };
                // The sea, in the picture. A picture narrower than the viewer
                // sits against the conversation, as a Short against its
                // comments, the viewer's surround on its other side.
                QCOMPARE(at(QPointF(video.center().x(), video.top() + video.height() * 0.85)), sea.rgb());
                const bool pillarboxed = video.width() < room.width() - 1;
                QCOMPARE(pillarboxed, format.canvas.width() <= format.canvas.height());
                if (pillarboxed) {
                    const int pictureRight = window.m_viewer->mapTo(&window, QPoint()).x() + int(std::ceil(video.right()));
                    QVERIFY2(chatRect.left() - pictureRight <= 16, qPrintable(QStringLiteral("%1: %2 px").arg(label).arg(chatRect.left() - pictureRight)));
                    QCOMPARE(at(QPointF(video.right() - 4, video.top() + video.height() * 0.85)), sea.rgb());
                    QCOMPARE(at(QPointF(video.left() - 4, video.center().y())), theme.viewerSurround.rgb());
                }
                // The conversation's column: the well, under its header with
                // its close button, and nothing in it wider than it.
                QWidget *column = window.findChild<QWidget *>(QStringLiteral("assistantPanel"));
                QCOMPARE(image.pixelColor((column->mapTo(&window, QPoint(2, column->height() - 2))) * ratio).rgb(), theme.well.rgb());
                QToolButton *close = chatArea->findChild<QToolButton *>(QStringLiteral("assistantClose"));
                QVERIFY(close && close->isVisible());
                QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
                QVERIFY(window.m_chatToggle->isChecked());
                // Its next steps show once, in it, not again over the timeline.
                QVERIFY(window.m_chatNextSteps->isVisible() && !window.m_aiBar->isVisible());
                QCOMPARE(amberPixels(image), 0);
                const QString scheme = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");
                if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("editor-%1-%2.png").arg(label, scheme))));
                // With the conversation closed from the toolbar, the viewer
                // runs to the window's edge and the picture recentres in it.
                if (format.canvas.width() < format.canvas.height()) {
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(!chatArea->isVisible() && !window.m_chatToggle->isChecked());
                    // Closed, the timeline's bar stands in for it with the next steps.
                    QVERIFY(window.m_aiBar->isVisible());
                    QCOMPARE(viewerArea->mapTo(&window, QPoint(viewerArea->width(), 0)).x(), window.width());
                    QVERIFY(std::abs(window.m_viewer->videoRect().center().x() - window.m_viewer->width() / 2.0) < 1);
                    const QImage closed = window.grab().toImage();
                    QCOMPARE(amberPixels(closed), 0);
                    if (!shots.isEmpty()) QVERIFY(closed.save(QDir(shots).filePath(QStringLiteral("editor-%1-closed-%2.png").arg(label, scheme))));
                    // Opened again, the picture is back against it.
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(chatArea->isVisible());
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                window.close();
            }
        }
    }

    // Push buttons and pop-up buttons take focus from Tab alone, as macOS's
    // do, so a click leaves no accent behind. Tab rings the control in the
    // accent at 35%, 3px outside its bezel, and the ring moves with the
    // focus. A field being typed in is ringed however it got focus.
    void focusRingsFollowTheKeyboardAndTyping() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            QDialog dialog;
            auto *layout = new QHBoxLayout(&dialog);
            layout->setContentsMargins(12, 12, 12, 12);
            auto *field = new QLineEdit;
            auto *reject = new QPushButton(QStringLiteral("Reject"));
            auto *choice = new QComboBox;
            choice->addItems({QStringLiteral("Preview"), QStringLiteral("Full")});
            auto *seconds = new StepperSpinBox;
            auto *details = new QPushButton(QStringLiteral("Details"));
            layout->addWidget(reject);
            layout->addWidget(field);
            layout->addWidget(choice);
            layout->addWidget(seconds);
            layout->addWidget(details);
            dialog.show();
            QVERIFY(QTest::qWaitForWindowActive(&dialog));
            QCOMPARE(reject->focusPolicy(), Qt::TabFocus);
            QCOMPARE(choice->focusPolicy(), Qt::TabFocus);
            // The focus a window gives its first control as it opens is no
            // keyboard navigation, so that button is not ringed.
            QCOMPARE(QApplication::focusWidget(), reject);
            QVERIFY(!dialog.findChild<QWidget *>(QStringLiteral("focusHalo")));
            const auto ringAround = [&](QWidget *control) {
                QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
                const auto halos = dialog.findChildren<QWidget *>(QStringLiteral("focusHalo"));
                QCOMPARE(halos.size(), 1);
                QVERIFY(halos.constFirst()->isVisible());
                QCOMPARE(halos.constFirst()->geometry(), control->geometry().adjusted(-3, -3, 3, 3));
                // The ring's middle, beside the bezel, is the accent at 35% on the dialog.
                const QImage image = dialog.grab().toImage();
                const QColor ring = image.pixelColor(QPoint(control->x() - 2, control->geometry().center().y()) * image.devicePixelRatio());
                const QColor expected = mixColor(theme.dialog, theme.accent, theme.focusHalo.alphaF());
                QVERIFY2(std::abs(ring.red() - expected.red()) <= 3 && std::abs(ring.green() - expected.green()) <= 3 &&
                             std::abs(ring.blue() - expected.blue()) <= 3,
                         qPrintable(ring.name() + QStringLiteral(" for ") + expected.name()));
            };
            // A click puts the caret in the field, which is ringed.
            QTest::mouseClick(field, Qt::LeftButton);
            QVERIFY(field->hasFocus());
            ringAround(field);
            // A click on a button takes no focus, so the field keeps it.
            QTest::mouseClick(reject, Qt::LeftButton);
            QVERIFY(!reject->hasFocus());
            ringAround(field);
            // A pop-up is a button's bezel, not a field's.
            const QImage unfocused = dialog.grab().toImage();
            QVERIFY(popUpHasButtonBezel(unfocused, choice, theme.button));
            // Tab rings the pop-up, the stepper's field, then the buttons.
            QTest::keyClick(field, Qt::Key_Tab);
            QVERIFY(choice->hasFocus());
            ringAround(choice);
            // The ring alone shows a pop-up's focus: its edge stays as it was.
            {
                const QImage image = dialog.grab().toImage();
                const QPoint edge = QPoint(choice->x(), choice->geometry().center().y()) * image.devicePixelRatio();
                QCOMPARE(image.pixelColor(edge).rgb(), unfocused.pixelColor(edge).rgb());
            }
            QTest::keyClick(choice, Qt::Key_Tab);
            QVERIFY(seconds->hasFocus());
            ringAround(seconds);
            QTest::keyClick(seconds, Qt::Key_Tab);
            QVERIFY(details->hasFocus());
            ringAround(details);
            QTest::keyClick(details, Qt::Key_Tab);
            QVERIFY(reject->hasFocus());
            ringAround(reject);
            // Back to the field: the ring moves to it.
            field->setFocus();
            ringAround(field);
            // A multi-line field is ringed as it is typed in too, around its
            // 3px bezel; a read-only one, which takes no typing, is not.
            auto *notes = new QPlainTextEdit;
            auto *transcript = new QTextEdit;
            transcript->setReadOnly(true);
            layout->addWidget(notes);
            layout->addWidget(transcript);
            notes->show();
            transcript->show();
            QCoreApplication::processEvents();
            QTest::mouseClick(notes->viewport(), Qt::LeftButton);
            QVERIFY(notes->hasFocus());
            ringAround(notes);
            QCOMPARE(notes->property("bezelRadius").toInt(), 3);
            QTest::mouseClick(transcript->viewport(), Qt::LeftButton);
            QVERIFY(transcript->hasFocus());
            QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
            QVERIFY(!dialog.findChild<QWidget *>(QStringLiteral("focusHalo")));
            // A combo box that takes typing stays a field.
            auto *typed = new QComboBox;
            typed->setEditable(true);
            typed->addItem(QStringLiteral("Kerf Cloud"));
            layout->addWidget(typed);
            typed->show();
            QCoreApplication::processEvents();
            QVERIFY(!popUpHasButtonBezel(dialog.grab().toImage(), typed, theme.button));
        }
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
            // The selected mask's name is white over the (blue) picture.
            const QRect nameArea = QRectF(corner + QPointF(3, -16), QSizeF(16, 13)).toAlignedRect();
            int lightest = 0;
            for (int y = nameArea.top(); y <= nameArea.bottom(); ++y)
                for (int x = nameArea.left(); x <= nameArea.right(); ++x)
                    lightest = std::max(lightest, view.pixelColor(QPoint(x, y) * ratio).lightness());
            QVERIFY2(lightest > 240, qPrintable(QString::number(lightest)));
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
            // The position is cut out of the marked range, a pixel of the panel each side.
            QCOMPARE(bar.pixelColor(QPoint(barX(2.0) - 2, 9) * barRatio).rgb(), theme.panel.rgb());
            QCOMPARE(bar.pixelColor(QPoint(barX(2.0) + 1, 9) * barRatio).rgb(), theme.panel.rgb());
            const QImage source = monitor.grab().toImage();
            QCOMPARE(source.pixelColor(QPoint(2, 2) * source.devicePixelRatio()), theme.viewerSurround);
            QCOMPARE(amberPixels(source), 0);
            save(source, QStringLiteral("source-%1").arg(name));
            monitor.close();

            // Bin tiles and their badges, redrawn for the new appearance.
            // Sound is the teal tile; a picture tile with no picture has an edge.
            QCOMPARE(tilePixel(3, QPoint(2, 60)), theme.audioTile);
            QCOMPARE(tilePixel(2, QPoint(56, 60)), theme.tileEmpty);
            QCOMPARE(tilePixel(2, QPoint(0, 40)), theme.tileEmptyBorder);
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
            const QImage browser = effects.grab().toImage();
            // The category pop-up is a button's bezel, as every pop-up is.
            QVERIFY(popUpHasButtonBezel(browser, effects.m_category, theme.button));
            save(browser, QStringLiteral("effects-%1").arg(name));
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
