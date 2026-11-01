#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
#include "player_overlay.h"
#include "device_frame.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
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
#include <QMenuBar>
#include <QMimeData>
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
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QScreen>
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
    // Open Kerf's conversation in the Program monitor's stage.
    static void showAssistant(MainWindow &window) {
        window.setChatOpen(true);
        QCoreApplication::processEvents();
    }

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

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
        // Preferences go to a file of this run's own, apart from the
        // editor's. Windows' registry keeps nothing for an application with
        // no organization, so the tests name one.
        static QTemporaryDir settings;
        QVERIFY(settings.isValid());
        QSettings::setDefaultFormat(QSettings::IniFormat);
        QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, settings.path());
        QCoreApplication::setOrganizationName(QStringLiteral("Industrial Kerf Tests"));
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
        // In a short viewer the wide picture's laptop fills its height.
        QVERIFY(viewer.stageLayout().body.top() < 20.0);
        viewer.show();
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QTest::mouseMove(&viewer, QPoint(400, 80));
        QVERIFY(!scrubs.isEmpty());
        QVERIFY(std::abs(scrubs.constLast()[0].toDouble() - 0.5) < 0.005);
        QVERIFY(nearerScrubFrame(1.0, true, 2.0, 3.0, 24.0));
        QVERIFY(!nearerScrubFrame(2.5, true, 0.2, 3.0, 24.0));
        QVERIFY(nearerScrubFrame(0.0, false, 9.0, 1.0, 24.0));
    }

    // A portrait picture shows in a phone, as a simulator shows an app: a
    // generic phone drawn in code, its screen 9:18.5 inside an even bezel,
    // the picture across the screen's width and centred on its height, the
    // player's controls inside the picture. A wide or square picture never
    // shows in one; Off is the plain card. Island and punch-hole differ, and
    // safe zones outline where a vertical-video app covers the picture.
    void portraitPictureShowsInAPhone() {
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1080, 1920);
        QCOMPARE(viewer.devicePreview(), DevicePreview::Island);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device);
        QCOMPARE(viewer.videoRect(), stage.card);
        // The phone fills the stage's height, inside the stage's inset (in
        // this short stage, the least: its screen is under 240 points even so).
        const int inset = viewer.stageInset(ViewerWidget::Device::Phone);
        QCOMPARE(inset, ViewerWidget::kLeastStageInset);
        QVERIFY(std::abs(stage.body.height() - (viewer.height() - 2 * inset)) < 1);
        QVERIFY(std::abs(stage.body.top() - inset) < 1);
        // As much room under it as over it.
        QVERIFY(std::abs((viewer.height() - stage.body.bottom()) - stage.body.top()) <= 1);
        // Where the stage is tall enough, the card's 16 over and under it.
        {
            viewer.resize(900, 620);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::Phone), ViewerWidget::kStageMargin);
            const ViewerWidget::StageLayout tall = viewer.stageLayout();
            QVERIFY(std::abs(tall.body.top() - ViewerWidget::kStageMargin) < 1);
            QVERIFY(std::abs((viewer.height() - tall.body.bottom()) - ViewerWidget::kStageMargin) <= 1);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::Laptop), ViewerWidget::kStageMargin);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::None), ViewerWidget::kStageMargin);
            viewer.resize(900, 460);
        }
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - deviceBodyAspect()) < 0.01);
        const DeviceGeometry device = deviceGeometry(stage.body, 9.0 / 16.0);
        QCOMPARE(device.screen, stage.screen);
        QVERIFY(std::abs(device.bodyRadius - kDeviceBodyRadius * stage.body.width()) < 0.01);
        QVERIFY(std::abs(device.bezel - kDeviceBezel * stage.body.width()) < 0.01);
        QVERIFY(std::abs(device.screenRadius - (device.bodyRadius - device.bezel)) < 0.01);
        QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kDeviceScreenAspect) < 0.01);
        // The picture: 9:16, across the screen, centred, black above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() - stage.screen.top() > deviceStatusBar(device).height());
        // The player's controls, the bar and the time stay on the picture.
        QVERIFY(stage.card.contains(viewer.overlay()->controlsBounds()));
        QVERIFY(viewer.overlay()->compact());
        // The camera: a pill for the island, a dot for the punch-hole, both
        // in the black over the picture.
        const QRectF island = deviceCamera(device, DevicePreview::Island);
        const QRectF hole = deviceCamera(device, DevicePreview::PunchHole);
        QVERIFY(island.width() > 3 * hole.width());
        QVERIFY(std::abs(hole.width() - hole.height()) < 0.01);
        for (const QRectF &camera : {island, hole}) {
            QVERIFY(stage.screen.contains(camera));
            QVERIFY(camera.bottom() < stage.card.top());
        }
        // Drawn: the graphite frame and the screen's black, in either appearance.
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(270, 480)), 0.0);
        const auto render = [&viewer] { return viewer.grab().toImage(); };
        const QImage withIsland = render();
        const qreal ratio = withIsland.devicePixelRatio();
        const auto at = [ratio](const QImage &image, const QPointF &point) { return image.pixelColor((point * ratio).toPoint()).rgb(); };
        QCOMPARE(at(withIsland, QPointF(stage.body.center().x(), stage.body.top() + device.bezel / 2.0 + 1)), editorTheme().deviceFrame.rgb());
        QCOMPARE(at(withIsland, QPointF(stage.screen.left() + device.screenRadius, (stage.screen.bottom() + stage.card.bottom()) / 2.0)),
                 editorTheme().deviceScreen.rgb());
        QCOMPARE(at(withIsland, stage.body.topLeft() + QPointF(1, 1)), editorTheme().viewerSurround.rgb());
        viewer.setDevicePreview(DevicePreview::PunchHole);
        QCOMPARE(viewer.stageLayout().card, stage.card);
        const QImage withHole = render();
        QVERIFY(withIsland != withHole);
        // Safe zones: off until asked for, then a wash over the picture's
        // right-hand column, its bottom band and its top.
        QVERIFY(!viewer.safeZonesShown());
        const SafeZones zones = safeZones(stage.card);
        QVERIFY(stage.card.contains(zones.top) && stage.card.contains(zones.actions) && stage.card.contains(zones.caption));
        QVERIFY(zones.actions.left() > stage.card.center().x() && zones.caption.top() > stage.card.center().y());
        viewer.setSafeZones(true);
        const QImage withZones = render();
        const QPointF inColumn(zones.actions.center().x(), zones.actions.top() + 6);
        const QPointF inBand(zones.caption.right() - 6, zones.caption.center().y());
        for (const QPointF &point : {inColumn, inBand})
            QVERIFY(QColor(at(withZones, point)).lightness() > QColor(at(withHole, point)).lightness());
        viewer.setSafeZones(false);
        QCOMPARE(render(), withHole);
        // Off: the plain card, larger than the phone's picture.
        viewer.setDevicePreview(DevicePreview::Off);
        stage = viewer.stageLayout();
        QVERIFY(!stage.device && stage.screen.isEmpty());
        QCOMPARE(stage.body, stage.card);
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(stage.card.width() > viewer.stageLayout().card.width() - 1);
        // Wide and square pictures never show in a phone: a wide one shows
        // in a laptop, a square one on the plain card.
        viewer.setDevicePreview(DevicePreview::Island);
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1080)}) {
            viewer.setProjectInfo(canvas.width(), canvas.height());
            stage = viewer.stageLayout();
            QVERIFY(stage.kind != ViewerWidget::Device::Phone);
            QCOMPARE(stage.kind, canvas.width() > canvas.height() ? ViewerWidget::Device::Laptop : ViewerWidget::Device::None);
            QVERIFY(!viewer.overlay()->compact());
        }
        QCOMPARE(viewer.stageLayout().body, viewer.stageLayout().card);
    }

    // A wide picture shows in a laptop, the phone's companion: a generic
    // laptop of today drawn in code in a warm stone metal, its lid black
    // glass in a thin rim, its screen 16:10 inside a thin even bezel (a
    // little thicker over it), the 16:9 picture across the screen with thin
    // black bars above and below, the camera's notch in the bar over it,
    // the player's controls inside the screen, and a wider, thin base under
    // the lid. Off is the plain card. The conversation is level with the laptop's body and as
    // tall, and the picture keeps at least 90% of the plain card's width.
    void widePictureShowsInALaptop() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1920, 1080);
        QCOMPARE(viewer.laptopPreview(), LaptopPreview::Laptop);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device);
        QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
        QCOMPARE(viewer.videoRect(), stage.card);
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - laptopBodyAspect()) < 0.01);
        const LaptopGeometry laptop = laptopGeometry(stage.body, 16.0 / 9.0);
        QCOMPARE(laptop.screen, stage.screen);
        // The lid: an even bezel of 1.8% of its width, a little more over
        // the screen; the screen 16:10; the base 108% of the lid wide and thin,
        // and the body the stage lays out exactly as wide as the base.
        QVERIFY(std::abs(laptop.bezel - kLaptopBezel * laptop.lid.width()) < 0.01);
        QVERIFY(kLaptopBaseWidth >= 1.08 && kLaptopBaseWidth <= 1.14);
        QVERIFY(std::abs(laptop.base.left() - laptop.body.left()) < 0.01 && std::abs(laptop.base.right() - laptop.body.right()) < 0.01);
        QVERIFY(laptop.body.adjusted(-0.01, -0.01, 0.01, 0.01).contains(laptop.base));
        QVERIFY(laptop.topBezel > laptop.bezel);
        QVERIFY(std::abs((laptop.screen.left() - laptop.lid.left()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.right() - laptop.screen.right()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.bottom() - laptop.screen.bottom()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs(laptop.screen.width() / laptop.screen.height() - 16.0 / 10.0) < 0.001);
        QVERIFY(std::abs(laptop.base.width() / laptop.lid.width() - kLaptopBaseWidth) < 0.001);
        QVERIFY(laptop.base.height() >= 0.029 * laptop.lid.height() && laptop.base.height() <= 0.04 * laptop.lid.height());
        QCOMPARE(laptop.base.top(), laptop.lid.bottom());
        QCOMPARE(laptop.body.bottom(), laptop.base.bottom());
        QVERIFY(std::abs(laptop.lid.center().x() - stage.body.center().x()) < 0.01);
        // The picture: 16:9, across the screen, centred, thin bars above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() > stage.screen.top() && stage.card.bottom() < stage.screen.bottom());
        // The notch hangs from the bezel into the bar over the picture,
        // centred, never over the picture, the camera in it.
        QCOMPARE(laptop.notch.top(), laptop.screen.top());
        QVERIFY(laptop.notch.bottom() < stage.card.top());
        QVERIFY(std::abs(laptop.notch.center().x() - laptop.lid.center().x()) < 0.01);
        const QRectF camera = laptopCamera(laptop);
        QVERIFY(laptop.notch.contains(camera));
        // The player's controls, in a video player's arrangement, inside the screen.
        QVERIFY(!viewer.overlay()->compact());
        QCOMPARE(viewer.overlay()->rect(), stage.card);
        QVERIFY(stage.screen.contains(viewer.overlay()->controlsBounds()));
        // Drawn: the lid's metal rim round its black glass, the screen's
        // black in the bars, the notch, the base's lit top face, and the
        // stage past the lid's corner.
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(480, 270)), 0.0);
        const QImage image = viewer.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](const QPointF &point) { return image.pixelColor((point * ratio).toPoint()).rgb(); };
        QCOMPARE(at(QPointF(std::ceil(laptop.lid.left()) + 0.25, laptop.lid.center().y())), editorTheme().laptopShell.rgb());
        QCOMPARE(at(QPointF((laptop.lid.left() + laptop.rim + laptop.screen.left()) / 2.0, laptop.lid.center().y())), editorTheme().laptopBezel.rgb());
        QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.top() + stage.card.top()) / 2.0)), editorTheme().deviceScreen.rgb());
        QCOMPARE(at(QPointF(laptop.notch.left() + 2.0, laptop.notch.top() + 1.0)), editorTheme().laptopBezel.rgb());
        QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.bottom() + stage.card.bottom()) / 2.0)), editorTheme().deviceScreen.rgb());
        QCOMPARE(at(QPointF(laptop.base.left() + laptop.base.width() * 0.25, laptop.base.top() + 3.0)), editorTheme().laptopDeck.rgb());
        // The lid's corners are round: no metal at the very corner.
        QVERIFY(at(laptop.lid.topLeft() + QPointF(1, 1)) != editorTheme().laptopShell.rgb());
        // Off: the plain card.
        viewer.setLaptopPreview(LaptopPreview::Off);
        stage = viewer.stageLayout();
        QVERIFY(!stage.device && stage.screen.isEmpty());
        QCOMPARE(stage.body, stage.card);
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 16.0 / 9.0) < 0.001);
        // Too narrow for the row of controls and the time over the length:
        // the player's compact arrangement, as on a portrait card.
        viewer.setLaptopPreview(LaptopPreview::Laptop);
        viewer.resize(400, 200);
        (void)viewer.grab();
        QVERIFY(viewer.stageLayout().card.width() < ViewerWidget::kCompactPlayerWidth);
        QVERIFY(viewer.overlay()->compact());
        viewer.resize(900, 460);
        (void)viewer.grab();
        QVERIFY(!viewer.overlay()->compact());
        // A portrait picture keeps its phone whatever the laptop's choice.
        viewer.setProjectInfo(1080, 1920);
        QCOMPARE(viewer.stageLayout().kind, ViewerWidget::Device::Phone);
    }

    // The laptop's (and the phone's) frame is drawn once for its size,
    // appearance and pixel ratio, then only blitted: repainting the same
    // size, as playback does for every frame, never draws it again.
    void deviceFrameIsDrawnOnceForItsSize() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        setEditorColorScheme(ColorScheme::Dark);
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1920, 1080);
        QCOMPARE(viewer.deviceFrameRenders(), 0);
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 1);
        // New frames of video, a moved playhead and its controls: blitted.
        for (int index = 0; index < 5; ++index) {
            viewer.setFrame(testing::footageFrame(index % 2 ? testing::Shot::HarborWide : testing::Shot::DronePass, QSize(480, 270)), index / 24.0);
            (void)viewer.grab();
        }
        viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 1);
        // A new size draws it again, once.
        viewer.resize(1000, 520);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 2);
        // So does the other appearance.
        setEditorColorScheme(ColorScheme::Light);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 3);
        // The phone's frame is cached the same way.
        viewer.setProjectInfo(1080, 1920);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 4);
        // The plain card has no frame to draw.
        viewer.setDevicePreview(DevicePreview::Off);
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 4);
        viewer.setDevicePreview(DevicePreview::Island);
    }

    // The phone and safe zones are this Mac's preference, never the
    // project's: the rail and Window ▸ Viewer set them, for a portrait
    // picture only, and the next editor opens with them.
    void devicePreviewIsRememberedOnThisMac() {
        {
            MainWindow window{QString(), QString()};
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // A wide picture: the laptop on the rail, no safe zones.
            QVERIFY(window.m_rail->deviceButton()->isVisible() && !window.m_rail->safeZonesButton()->isVisible());
            QCOMPARE(window.m_rail->deviceButton()->glyph(), RailButton::Glyph::Laptop);
            QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1080);
            timeline.insert(QStringLiteral("height"), 1920);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(snapshot);
            QCoreApplication::processEvents();
            RailButton *device = window.m_rail->deviceButton();
            RailButton *safe = window.m_rail->safeZonesButton();
            QVERIFY(device->isVisible() && safe->isVisible());
            QVERIFY(device->isChecked() && !safe->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Phone"));
            QCOMPARE(device->glyph(), RailButton::Glyph::Phone);
            QVERIFY(window.m_viewer->stageLayout().device);
            // The panel is level with the phone's body and as tall.
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            QCOMPARE(window.m_chat->geometry().top(), qRound(stage.body.top()));
            QVERIFY(std::abs(window.m_chat->height() - stage.body.height()) <= 1);
            // Window ▸ Viewer offers the same choices.
            QCOMPARE(window.m_deviceActions.size(), 3);
            QVERIFY(window.m_deviceActions.constFirst()->isChecked());
            window.m_deviceActions.at(1)->trigger();
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::PunchHole);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/devicePreview")).toString(), QStringLiteral("punchhole"));
            QTest::mouseClick(safe, Qt::LeftButton, {}, safe->discRect().center());
            QVERIFY(window.m_viewer->safeZonesShown() && safe->isChecked() && window.m_safeZonesAction->isChecked());
            QCOMPARE(QSettings().value(QStringLiteral("viewer/safeZones")).toBool(), true);
            window.m_deviceActions.at(2)->trigger();
            QVERIFY(!window.m_viewer->stageLayout().device && !device->isChecked());
            // Off keeps its name (its glyph is slashed), as the menu says
            // "Phone · Off"; no word of its own.
            QCOMPARE(device->label(), QStringLiteral("Phone"));
            window.m_deviceActions.at(1)->trigger();
            // Nothing of it reaches the project.
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("device")));
            window.close();
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::PunchHole);
        QVERIFY(window.m_viewer->safeZonesShown());
        QVERIFY(window.m_deviceActions.at(1)->isChecked() && window.m_safeZonesAction->isChecked());
    }

    // A wide picture shows in a laptop by default, this Mac's preference
    // like the phone: the rail's Laptop button and Window ▸ Viewer choose
    // Laptop or Off, and the next editor opens with it. At a laptop's
    // window the conversation is level with the laptop's body (lid top to
    // base bottom) and as tall, the picture keeps at least 90% of the width
    // the plain card has in the same stage, and with the conversation closed
    // the laptop is centred alone.
    void laptopPreviewIsRememberedOnThisMac() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer/laptopPreview"));
        QSettings().remove(QStringLiteral("viewer/chatOpen"));
        QSettings().remove(QStringLiteral("viewer/chatWidth"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("viewer/laptopPreview"));
            QSettings().remove(QStringLiteral("viewer/chatOpen"));
        });
        {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            ViewerWidget *viewer = window.m_viewer;
            QCOMPARE(window.m_canvasSize, QSize(1920, 1080));
            RailButton *device = window.m_rail->deviceButton();
            QVERIFY(device->isVisible() && device->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Laptop"));
            QCOMPARE(device->glyph(), RailButton::Glyph::Laptop);
            const ViewerWidget::StageLayout stage = viewer->stageLayout();
            QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
            QVERIFY(!stage.overlay);
            // The conversation: the laptop body's top and height.
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(stage.panel.top(), qRound(stage.body.top()));
            QVERIFY2(std::abs(stage.panel.height() - stage.body.height()) <= 1,
                     qPrintable(QStringLiteral("%1 for %2").arg(stage.panel.height()).arg(stage.body.height())));
            QVERIFY(stage.panel.height() >= ViewerWidget::kMinChatHeight);
            // Laptop, rail, conversation: one group, centred.
            const int leftSpace = qRound(stage.body.left());
            const int rightSpace = viewer->width() - stage.panel.right() - 1;
            QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1 | %2").arg(leftSpace).arg(rightSpace)));
            QCOMPARE(stage.rail.left() - qRound(stage.body.right()), viewer->stageGap());
            // Measured from the laptop's widest part, its base: the stage's
            // margin on the left, the stage's gap before the rail.
            const auto clearOfEdges = [viewer](const ViewerWidget::StageLayout &layout) {
                const QRectF base = laptopGeometry(layout.body, 16.0 / 9.0).base;
                QVERIFY(layout.body.adjusted(-0.01, -0.01, 0.01, 0.01).contains(base));
                QVERIFY2(base.left() >= ViewerWidget::kStageMargin - 0.5, qPrintable(QString::number(base.left())));
                QVERIFY2(layout.rail.left() - base.right() >= viewer->stageGap() - 1.0,
                         qPrintable(QStringLiteral("%1 to %2").arg(base.right()).arg(layout.rail.left())));
            };
            clearOfEdges(stage);
            // The picture keeps 90% of the plain card's width in this stage.
            const ViewerWidget::StageLayout plain = viewer->stageLayout(ViewerWidget::Device::None);
            QVERIFY2(stage.card.width() >= 0.9 * plain.card.width(),
                     qPrintable(QStringLiteral("%1 of %2").arg(stage.card.width()).arg(plain.card.width())));
            // The controls inside the screen.
            QVERIFY(stage.screen.contains(viewer->overlay()->controlsBounds()));
            // Window ▸ Viewer has Laptop and Off; Off is the plain card, level
            // with the conversation and as tall too.
            QCOMPARE(window.m_laptopActions.size(), 2);
            QVERIFY(window.m_laptopActions.at(0)->isChecked() && !window.m_laptopActions.at(1)->isChecked());
            window.m_laptopActions.at(1)->trigger();
            QCoreApplication::processEvents();
            QCOMPARE(viewer->laptopPreview(), LaptopPreview::Off);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/laptopPreview")).toString(), QStringLiteral("off"));
            QVERIFY(!device->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Laptop"));
            const ViewerWidget::StageLayout card = viewer->stageLayout();
            QVERIFY(!card.device);
            QCOMPARE(card.card, plain.card);
            QCOMPARE(window.m_chat->geometry(), card.panel);
            QCOMPARE(card.panel.top(), qRound(card.card.top()));
            QVERIFY(std::abs(card.panel.height() - card.card.height()) <= 1);
            window.m_laptopActions.at(0)->trigger();
            QCoreApplication::processEvents();
            QCOMPARE(viewer->stageLayout().card, stage.card);
            // Closed: the laptop centred alone.
            window.m_chatToggle->click();
            QCoreApplication::processEvents();
            const QRectF alone = viewer->stageLayout().body;
            QVERIFY(std::abs(alone.center().x() - viewer->width() / 2.0) <= 1);
            // As much room over it as under it, the card's 16: centred in
            // the stage, as a card is.
            {
                const double over = alone.top(), under = viewer->height() - alone.bottom();
                QVERIFY2(std::abs(over - under) <= 1, qPrintable(QStringLiteral("%1 over, %2 under").arg(over).arg(under)));
                QVERIFY2(std::abs(over - ViewerWidget::kStageMargin) <= 1, qPrintable(QStringLiteral("%1 over").arg(over)));
            }
            QVERIFY(alone.width() >= stage.body.width() - 0.5);
            clearOfEdges(viewer->stageLayout());
            window.m_chatToggle->click();
            window.m_laptopActions.at(1)->trigger();
            // Nothing of it reaches the project.
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("laptop")));
            window.close();
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->laptopPreview(), LaptopPreview::Off);
        QVERIFY(window.m_laptopActions.at(1)->isChecked());
    }

    // The transport lies on the picture as a video player's controls: shown
    // while paused, faded out a moment after the pointer stops while
    // playing, back with a move; the play button and the bar run the
    // editor's own playback and seeking; and the time reads on a white frame.
    void playerControlsLieOnThePicture() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        PlayerOverlay *overlay = viewer->overlay();
        overlay->setFadeDuration(0);
        overlay->setIdleTimeout(40);
        QCOMPARE(overlay->rect(), viewer->videoRect());
        QVERIFY(!overlay->compact());
        // Paused: shown, with the time over the length.
        QVERIFY(overlay->shown());
        QCOMPARE(overlay->opacity(), 1.0);
        QCOMPARE(overlay->timecodeText(), QStringLiteral("00:00:00:00 / 00:00:18:00"));
        QVERIFY(viewer->videoRect().contains(overlay->controlsBounds()));
        QVERIFY(overlay->controlTip(PlayerOverlay::Control::Play).startsWith(QStringLiteral("Play")));
        // The play button asks for playback.
        QTest::mouseClick(viewer, Qt::LeftButton, {}, overlay->controlRect(PlayerOverlay::Control::Play).center().toPoint());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("play"));
        // Playing: hidden a moment after the pointer's last move...
        QTest::mouseMove(viewer, viewer->videoRect().center().toPoint());
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", true}});
        QVERIFY(overlay->playing());
        QTRY_VERIFY(!overlay->shown());
        QCOMPARE(overlay->opacity(), 0.0);
        // ...back on the next move over the picture, and gone again after.
        QTest::mouseMove(viewer, viewer->videoRect().center().toPoint() + QPoint(5, 5));
        QVERIFY(overlay->shown());
        QTRY_VERIFY(!overlay->shown());
        // Paused again, it stays.
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", false}});
        QVERIFY(overlay->shown());
        // A click on the bar seeks to the frame under it, and a drag follows.
        const QRectF bar = overlay->barRect();
        const auto sendMouse = [viewer](QEvent::Type type, const QPointF &at, Qt::MouseButtons buttons) {
            QMouseEvent event(type, at, viewer->mapToGlobal(at), type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton, buttons,
                              Qt::NoModifier);
            QCoreApplication::sendEvent(viewer, &event);
        };
        sendMouse(QEvent::MouseButtonPress, QPointF(bar.left() + bar.width() * 0.5, bar.center().y()), Qt::LeftButton);
        QVERIFY(std::abs(window.m_playhead - 9.0) < 1.0 / 24.0);
        sendMouse(QEvent::MouseMove, QPointF(bar.left() + bar.width() * 0.25, bar.center().y()), Qt::LeftButton);
        sendMouse(QEvent::MouseButtonRelease, QPointF(bar.left() + bar.width() * 0.25, bar.center().y()), Qt::NoButton);
        QVERIFY(std::abs(window.m_playhead - 4.5) < 1.0 / 24.0);
        QCOMPARE(overlay->timecodeText(), QStringLiteral("00:00:04:12 / 00:00:18:00"));
        // The step buttons step one frame.
        QTest::mouseClick(viewer, Qt::LeftButton, {}, overlay->controlRect(PlayerOverlay::Control::StepForward).center().toPoint());
        QVERIFY(std::abs(window.m_playhead - (4.5 + 1.0 / 24.0)) < 0.001);
        // No strip of buttons and timecodes under the picture: the stage
        // below the card is empty.
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("viewerFrame"))->findChild<QLabel *>(QStringLiteral("transportTime")));

        // On a portrait card at a laptop's window, a short-video player's
        // arrangement: the cluster at the top left, the bar along the foot,
        // the time small over it, all on the picture.
        QJsonObject snapshot = showcaseSnapshot(4, QStringLiteral("clean"));
        QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
        timeline.insert(QStringLiteral("width"), 1080);
        timeline.insert(QStringLiteral("height"), 1920);
        snapshot.insert(QStringLiteral("timeline"), timeline);
        window.applySnapshot(snapshot);
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        for (const DevicePreview preview : {DevicePreview::Island, DevicePreview::Off}) {
            window.setDevicePreview(preview);
            QCoreApplication::processEvents();
            const QRectF card = viewer->videoRect();
            QVERIFY(overlay->compact());
            QVERIFY2(card.contains(overlay->controlsBounds()), qPrintable(QStringLiteral("%1 wide").arg(card.width())));
            QVERIFY(overlay->controlRect(PlayerOverlay::Control::Play).top() < card.top() + 8);
            QVERIFY(overlay->barRect().bottom() >= card.bottom() - 0.5);
            QVERIFY(overlay->controlRect(PlayerOverlay::Control::StepForward).right() < card.right());
            // The time fits whole: over the length where there is room, the
            // playhead's alone where there is not.
            const QFontMetricsF metrics(overlay->timeFont(11));
            QVERIFY(metrics.horizontalAdvance(overlay->timecodeText()) <= overlay->timecodeRect().width());
            QVERIFY(overlay->timecodeText().startsWith(QStringLiteral("00:00:04:13")));
        }
        window.setDevicePreview(DevicePreview::Island);
        window.close();

        // The time reads at 4.5:1 on a white frame, on the gradient's darkest part.
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            ViewerWidget plain;
            plain.resize(900, 520);
            plain.setProjectInfo(canvas.width(), canvas.height());
            QImage white(canvas / 4, QImage::Format_RGB32);
            white.fill(Qt::white);
            plain.setFrame(white, 0.0);
            PlayerOverlay *controls = plain.overlay();
            const QImage image = plain.grab().toImage();
            const QRectF time = controls->timecodeRect();
            const QFontMetricsF metrics(controls->timeFont(controls->compact() ? 11 : 12));
            const QPointF clear(time.left() + metrics.horizontalAdvance(controls->timecodeText()) + 6, time.center().y());
            QVERIFY(controls->rect().contains(clear));
            const QColor under = image.pixelColor((clear * image.devicePixelRatio()).toPoint());
            const QColor ink = laidOn(under, editorTheme().playerInk);
            QVERIFY2(contrastRatio(ink, under) >= 4.5, qPrintable(QStringLiteral("%1 on %2").arg(ink.name(), under.name())));
        }
    }

    // The pointer on the picture works as on a video player: skimming maps
    // across the picture (as the bar does), not across the stage, so the
    // frame skimmed is the one under the bar's tip; a click on the picture
    // plays or pauses and never seeks; off the picture nothing happens.
    void pictureSkimsAcrossItselfAndAClickPlays() {
        ViewerWidget viewer;
        viewer.resize(1063, 528);
        viewer.setStage(new QWidget, new QWidget);
        viewer.setChatOpen(true);
        viewer.setProjectInfo(1080, 1920);
        viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&viewer));
        const QRectF card = viewer.videoRect();
        QVERIFY(viewer.stageLayout().device);
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QSignalSpy ends(&viewer, &ViewerWidget::scrubEnded);
        QSignalSpy seeks(viewer.overlay(), &PlayerOverlay::seekRequested);
        QSignalSpy controls(viewer.overlay(), &PlayerOverlay::controlTriggered);
        QTest::mouseMove(&viewer, card.center().toPoint());
        QVERIFY(!scrubs.isEmpty());
        QVERIFY2(std::abs(scrubs.constLast()[0].toDouble() - 0.5) <= 0.02, qPrintable(QString::number(scrubs.constLast()[0].toDouble())));
        QTest::mouseMove(&viewer, QPoint(qRound(card.left() + card.width() * 0.25), qRound(card.center().y())));
        QVERIFY(std::abs(scrubs.constLast()[0].toDouble() - 0.25) <= 0.02);
        // Off the picture, on the stage beside the phone: the skim ends.
        const qsizetype skimmed = scrubs.size();
        QTest::mouseMove(&viewer, QPoint(8, qRound(card.center().y())));
        QCOMPARE(scrubs.size(), skimmed);
        QCOMPARE(ends.size(), 1);
        // A click on the picture plays; a click on the stage does nothing.
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, card.center().toPoint());
        QCOMPARE(controls.size(), 1);
        QCOMPARE(controls.constFirst()[0].value<PlayerOverlay::Control>(), PlayerOverlay::Control::Play);
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, QPoint(8, 8));
        QCOMPARE(controls.size(), 1);
        QVERIFY(seeks.isEmpty());
    }

    // The caption preview is lifted over the player's controls while they
    // show, as a video site lifts its captions, and drawn over the shade, so
    // the bar never runs through it; with the controls gone it is back at
    // its place near the foot. Safe zones keep the controls (all but the
    // bar) off the picture until the pointer comes over it. A mask's handle
    // under a control is the handle's, and the empty card has one play
    // button only, the player's.
    void captionsAndMasksKeepClearOfThePlayer() {
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(canvas.width(), canvas.height());
            viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
            PlayerOverlay *controls = viewer.overlay();
            controls->setFadeDuration(0);
            controls->setIdleTimeout(10);
            const QRectF video = viewer.videoRect();
            QCOMPARE(controls->opacity(), 1.0);
            const QRectF lifted = viewer.captionRect();
            QVERIFY(video.contains(lifted));
            QVERIFY2(lifted.bottom() <= controls->footTop() - 8.0 + 0.01,
                     qPrintable(QStringLiteral("%1 over %2").arg(lifted.bottom()).arg(controls->footTop())));
            QVERIFY(!lifted.intersects(controls->barReach()) && !lifted.intersects(controls->timecodeRect()));
            for (const PlayerOverlay::Control control : {PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack,
                                                         PlayerOverlay::Control::StepForward})
                QVERIFY(!lifted.intersects(controls->controlRect(control)));
            QVERIFY(contrastRatio(editorTheme().viewerCaptionText, laidOn(QColor(Qt::white), editorTheme().viewerCaption)) >= 4.5);
            // Playing, the controls fade: the caption goes back to its place.
            controls->setPlaying(true);
            QTRY_COMPARE(controls->opacity(), 0.0);
            const QRectF rest = viewer.captionRect();
            QVERIFY(rest.bottom() >= lifted.bottom());
            if (canvas.width() > canvas.height()) QVERIFY(rest.bottom() > lifted.bottom());
            QVERIFY(std::abs(rest.bottom() - (video.bottom() - std::max(12.0, video.height() * 0.08))) < 0.5);
            controls->setPlaying(false);
        }
        {
            // Safe zones: only the bar until the pointer is over the picture.
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1080, 1920);
            viewer.show();
            QVERIFY(QTest::qWaitForWindowExposed(&viewer));
            PlayerOverlay *controls = viewer.overlay();
            QVERIFY(controls->controlsShown());
            viewer.setSafeZones(true);
            QVERIFY(controls->controlsSuppressed() && !controls->controlsShown());
            QVERIFY(std::abs(controls->footTop() - (viewer.videoRect().bottom() - PlayerOverlay::kBarHoverHeight)) < 0.01);
            QTest::mouseMove(&viewer, viewer.videoRect().center().toPoint());
            QVERIFY(controls->controlsShown());
            QTest::mouseMove(&viewer, QPoint(4, 4));
            QVERIFY(!controls->controlsShown());
            viewer.setSafeZones(false);
            QVERIFY(!controls->controlsSuppressed());
        }
        {
            // A mask whose corner sits on the play button: pressing there
            // takes the corner, not the button.
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1920, 1080);
            viewer.show();
            QVERIFY(QTest::qWaitForWindowExposed(&viewer));
            const QRectF video = viewer.videoRect();
            const QPointF play = viewer.overlay()->controlRect(PlayerOverlay::Control::Play).center();
            const double fx = (play.x() - video.left()) / video.width();
            const double fy = (play.y() - video.top()) / video.height();
            viewer.setMasks(QStringLiteral("v1"),
                            QJsonArray{QJsonObject{{"name", "k1"}, {"mode", "add"}, {"rect", QJsonArray{fx, 0.2, 0.8, fy}}}},
                            QTransform::fromScale(1920, 1080));
            viewer.selectMask(QStringLiteral("k1"));
            QSignalSpy controls(viewer.overlay(), &PlayerOverlay::controlTriggered);
            QSignalSpy edits(&viewer, &ViewerWidget::editRequested);
            QTest::mousePress(&viewer, Qt::LeftButton, {}, play.toPoint());
            QTest::mouseMove(&viewer, (play + QPointF(20, -20)).toPoint());
            QTest::mouseRelease(&viewer, Qt::LeftButton, {}, (play + QPointF(20, -20)).toPoint());
            QVERIFY(controls.isEmpty());
            QCOMPARE(edits.size(), 1);
            QCOMPARE(edits.constFirst()[0].toJsonArray().first().toObject().value("op").toString(), QStringLiteral("mask-set"));
        }
        {
            // The empty card: two quiet lines, and no play disc of its own
            // over them (the player's play button is the only one).
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1920, 1080);
            const qreal ratio = viewer.grab().toImage().devicePixelRatio();
            // In the laptop, the screen's black; on the plain card, the canvas.
            for (const bool laptop : {true, false}) {
                viewer.setLaptopPreview(laptop ? LaptopPreview::Laptop : LaptopPreview::Off);
                const QImage image = viewer.grab().toImage();
                const QRectF video = viewer.videoRect();
                const QColor backdrop = laptop ? editorTheme().deviceScreen : editorTheme().viewerCanvas;
                for (int dy = -40; dy <= -26; dy += 2)
                    for (int dx = -14; dx <= 14; dx += 2)
                        QCOMPARE(image.pixelColor(((video.center() + QPointF(dx, dy)) * ratio).toPoint()).rgb(), backdrop.rgb());
            }
        }
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
            const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
            for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
                setEditorColorScheme(scheme);
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
        // Six tracks at their default heights fit at 1280x800 (three picture
        // tracks over three sound ones), under the ruler.
        QVERIFY2(window.m_timelineScroll->viewport()->height() - kRulerHeight >= 3 * kVideoTrackHeight + 3 * kTrackHeight,
                 qPrintable(QString::number(window.m_timelineScroll->viewport()->height())));
        // A snapshot arriving after the window is laid out: the title
        // strips' details show whole, not as an ellipsis.
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"duration", 8}, {"timeline", QJsonObject{{"name", "Cut"}, {"fps", 24}, {"tracks", QJsonArray{}}}}});
        QCoreApplication::processEvents();
        {
            auto *detail = window.findChild<QToolButton *>(QStringLiteral("viewerDetail"));
            QVERIFY(detail && detail->width() >= detail->fontMetrics().horizontalAdvance(detail->text()));
            auto *tracks = window.findChild<QLabel *>(QStringLiteral("timelineDetail"));
            QVERIFY(tracks && tracks->width() >= tracks->fontMetrics().horizontalAdvance(tracks->text()));
        }
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
        QVERIFY(window.m_viewer->overlay()->controlTip(PlayerOverlay::Control::Play).startsWith(QStringLiteral("Play (")));
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
            // Kerf's conversation starts open, in the Program monitor's stage.
            QVERIFY(window.chatOpen());
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

    // The Editing workspace reads as a Short does on YouTube's desktop page:
    // the Program monitor is the page, the widest panel of its row, and in
    // its stage the picture card, the action rail and Kerf's conversation
    // form one group, centred, the conversation as tall as the card and
    // level with it. The browsers and Effect Controls sit on its left, the
    // Source monitor is a tab behind it, and the Timeline runs the window's
    // full width under them.
    void editingShowsTheVideoWithItsConversationInItsStage() {
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

        // No dock of its own any more: it is open in the stage.
        QVERIFY(!panel("assistant"));
        QVERIFY(window.chatOpen());
        QAction *toggle = window.m_commands->action(QStringLiteral("window.assistant"));
        QVERIFY(toggle->isCheckable() && toggle->isChecked());
        QVERIFY(window.m_chatToggle && window.m_chatToggle->isChecked());
        QCOMPARE(window.m_chatToggle->defaultAction(), toggle);
        QCOMPARE(area("source"), area("program"));
        QVERIFY(panel("program")->isCurrentTab());
        for (const char *id : {"inspector", "effects", "presets"}) QCOMPARE(area(id), area("project"));
        QVERIFY(panel("project")->isCurrentTab());
        // Their column shows the four tabs' names in full.
        for (const char *id : {"project", "inspector", "effects", "presets"}) {
            auto *label = panel(id)->tabWidget()->findChild<ads::CElidingLabel *>();
            QVERIFY2(label && label->isVisible() && !label->isElided(), id);
        }
        // Every panel is open, and Window lists each one.
        for (const char *id : {"project", "source", "program", "timeline", "inspector", "effects", "presets"}) {
            QVERIFY2(!panel(id)->isClosed(), id);
            QAction *item = window.m_commands->action(QStringLiteral("window.%1").arg(QString::fromLatin1(id)));
            QVERIFY2(item && item->isCheckable() && item->isChecked(), id);
        }

        const QRect browser = frame(area("project"));
        const QRect viewer = frame(area("program"));
        const QRect timeline = frame(area("timeline"));
        // One row: the browsers, then the viewer to the window's edge.
        QVERIFY(browser.right() < viewer.left());
        QCOMPARE(viewer.right(), window.width() - 1);
        QCOMPARE(browser.top(), viewer.top());
        QVERIFY(viewer.width() > 2 * browser.width());
        QCOMPARE(timeline.left(), browser.left());
        QCOMPARE(timeline.right(), viewer.right());
        QVERIFY(timeline.top() > viewer.bottom());
        QVERIFY(viewer.height() > timeline.height());

        // The stage: the picture's laptop (or card), rail, conversation, one
        // group centred in it, the conversation level with the body and as
        // tall.
        const auto checkGroup = [&window](const char *what) {
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            const QRect body = stage.body.toAlignedRect();
            QVERIFY2(!stage.overlay, what);
            QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(window.m_rail->geometry(), stage.rail);
            QVERIFY2(window.m_chat->isVisible() && window.m_rail->isVisible(), what);
            QCOMPARE(stage.rail.left() - qRound(stage.body.right()), window.m_viewer->stageGap());
            QCOMPARE(stage.panel.left() - stage.rail.right() - 1, window.m_viewer->stageGap());
            const int leftSpace = body.left();
            const int rightSpace = window.m_viewer->width() - stage.panel.right() - 1;
            QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1: %2 | %3").arg(what).arg(leftSpace).arg(rightSpace)));
            QVERIFY2(std::abs(stage.body.center().y() - QRectF(stage.panel).center().y()) <= 1, what);
            QCOMPARE(stage.panel.top(), qRound(stage.body.top()));
            QVERIFY2(std::abs(stage.panel.height() - stage.body.height()) <= 1, what);
            QVERIFY2(stage.panel.height() >= ViewerWidget::kLeastChatHeight, what);
        };
        checkGroup("1440");
        // A wide picture keeps four fifths of its width with the
        // conversation closed: the conversation narrows for it from its
        // default, never under its least.
        const int chatWidth = window.m_chat->width();
        QVERIFY2(chatWidth >= ViewerWidget::kMinChatWidth && chatWidth <= ViewerWidget::kDefaultChatWidth, qPrintable(QString::number(chatWidth)));
        const QRectF card = window.m_viewer->videoRect();
        // A wider window widens the viewer; the browsers keep theirs, the
        // conversation widens back towards its own, and the picture takes
        // the rest.
        window.resize(1680, 900);
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("project")).width(), browser.width());
        QCOMPARE(frame(area("program")).width(), viewer.width() + 240);
        QVERIFY(window.m_chat->width() > chatWidth && window.m_chat->width() <= ViewerWidget::kDefaultChatWidth);
        QVERIFY(window.m_viewer->videoRect().width() >= card.width() - 0.5);
        checkGroup("1680");
        // A laptop's window: the conversation gives way before the picture,
        // which keeps at least four fifths of its width with the
        // conversation closed and stays the larger of the two; the gaps
        // narrow in a narrow stage.
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        {
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            QVERIFY(!stage.overlay);
            QCOMPARE(window.m_viewer->stageGap(), ViewerWidget::kNarrowStageGap);
            const double cardArea = stage.card.width() * stage.card.height();
            const double panelArea = double(stage.panel.width()) * stage.panel.height();
            QVERIFY2(cardArea >= panelArea, qPrintable(QStringLiteral("%1 < %2").arg(cardArea).arg(panelArea)));
            QVERIFY(stage.panel.width() >= ViewerWidget::kMinChatWidth);
            window.setChatOpen(false);
            QCoreApplication::processEvents();
            const double closedWidth = window.m_viewer->videoRect().width();
            window.setChatOpen(true);
            QCoreApplication::processEvents();
            QVERIFY(stage.card.width() >= ViewerWidget::kCardKeeps * closedWidth - 1 || stage.panel.width() == ViewerWidget::kMinChatWidth);
            checkGroup("1280");
        }
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        QCOMPARE(window.m_viewer->videoRect(), card);

        // The toolbar's bubble closes it; the picture is centred alone, the
        // rail on its right, and the browsers keep their width.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !window.m_chat->isVisible());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked() && !window.m_rail->chatButton()->isChecked());
        QVERIFY(std::abs(window.m_viewer->videoRect().center().x() - window.m_viewer->width() / 2.0) <= 1);
        QVERIFY(window.m_rail->isVisible());
        QCOMPARE(window.m_rail->x() - qRound(window.m_viewer->stageLayout().body.right()), ViewerWidget::kStageGap);
        QCOMPARE(frame(area("project")).width(), browser.width());
        // And opens it again as it was.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && window.m_rail->chatButton()->isChecked());
        QCOMPARE(window.m_viewer->videoRect(), card);
        checkGroup("reopened");

        // The header's close button, with its shortcut in its tip.
        ChatIconButton *close = window.m_chat->closeButton();
        QVERIFY(close->isVisible());
        QVERIFY(close->toolTip().startsWith(QStringLiteral("Close Kerf Assistant")));
        QTest::mouseClick(close, Qt::LeftButton);
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !toggle->isChecked());
        // ⇧⌘J opens it.
        QCOMPARE(toggle->shortcut(), QKeySequence(QStringLiteral("Ctrl+Shift+J")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && window.m_chatToggle->isChecked());
        // The rail's chat button, beside the picture, is the same switch.
        RailButton *rail = window.m_rail->chatButton();
        QVERIFY(rail->isChecked());
        QTest::mouseClick(rail, Qt::LeftButton, {}, rail->discRect().center());
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !toggle->isChecked() && !rail->isChecked());
        QTest::mouseClick(rail, Qt::LeftButton, {}, rail->discRect().center());
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && rail->isChecked());
        // Window ▸ Kerf Assistant too.
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !window.m_chatToggle->isChecked());
        // With the Source monitor in front, opening the conversation brings
        // the Program monitor, whose stage holds it, forward.
        panel("source")->setAsCurrentTab();
        QCoreApplication::processEvents();
        QVERIFY(!window.m_viewer->isVisible());
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(panel("program")->isCurrentTab() && window.m_chat->isVisible());
        // Open but out of sight behind the Source monitor, it reads as
        // closed: the timeline's bar stands in for it with Kerf's proposal,
        // and the first press of ⇧⌘J brings it back into sight.
        window.setConnection(QStringLiteral("ready"));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "p1"}, {"script", "remove v1c\n"},
                                          {"documentDiff", "-    clip v1c\n"}});
        panel("source")->setAsCurrentTab();
        QCoreApplication::processEvents();
        QVERIFY(window.m_viewer->chatOpen() && !window.chatOpen());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked());
        QVERIFY(window.m_aiBar->isVisible());
        QVERIFY(window.m_aiBarText->text().startsWith(QStringLiteral("Proposal")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(panel("program")->isCurrentTab() && window.chatOpen() && window.m_chat->isVisible());
        QVERIFY(toggle->isChecked() && window.m_chatToggle->isChecked());
        QVERIFY(!window.m_aiBar->isVisible());
        window.updateProposal(QJsonValue());
        toggle->trigger();
        // The AI bar's Details opens it.
        QToolButton *details = nullptr;
        for (QToolButton *button : window.m_aiBar->findChildren<QToolButton *>())
            if (button->text() == QStringLiteral("Details")) details = button;
        QVERIFY(details);
        details->click();
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && window.m_chatToggle->isChecked());

        // Dragging its leading edge widens it, within its bounds; the
        // picture gives the room, and the width is this Mac's, kept. (In a
        // window wide enough that the wide picture leaves it its own width.)
        window.resize(1680, 900);
        QCoreApplication::processEvents();
        const int ownWidth = window.m_chat->width();
        QCOMPARE(ownWidth, ViewerWidget::kDefaultChatWidth);
        const QRectF wideCard = window.m_viewer->videoRect();
        QWidget *handle = window.m_chat->resizeHandle();
        QVERIFY(handle->isVisible());
        QCOMPARE(handle->cursor().shape(), Qt::SizeHorCursor);
        QTest::mousePress(handle, Qt::LeftButton, {}, QPoint(2, 60));
        QTest::mouseMove(handle, QPoint(2 - 40, 60));
        QTest::mouseRelease(handle, Qt::LeftButton, {}, QPoint(2 - 40, 60));
        QCoreApplication::processEvents();
        QCOMPARE(window.m_chat->width(), ownWidth + 40);
        QCOMPARE(QSettings().value(QStringLiteral("viewer/chatWidth")).toInt(), ownWidth + 40);
        QVERIFY(window.m_viewer->videoRect().width() <= wideCard.width());
        checkGroup("wider");
        window.m_viewer->setChatWidth(10000);
        QCOMPARE(window.m_viewer->chatWidth(), ViewerWidget::kMaxChatWidth);
        // Shown that wide where the picture keeps its share beside it.
        QVERIFY(window.m_chat->width() <= ViewerWidget::kMaxChatWidth);
        window.resize(1920, 1000);
        QCoreApplication::processEvents();
        QCOMPARE(window.m_chat->width(), ViewerWidget::kMaxChatWidth);
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        window.m_viewer->setChatWidth(10);
        QCOMPARE(window.m_chat->width(), ViewerWidget::kMinChatWidth);
        // Whether it is open is kept too, never in the project.
        window.close();
        QCOMPARE(QSettings().value(QStringLiteral("viewer/chatOpen")).toBool(), true);
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

    // The new Editing arrangement takes the place of the Editing layout
    // saved before it, while a workspace of one's own saved then keeps its
    // arrangement, even one saved with the conversation's old dock (which
    // restoring leaves out); one from before today's panels gives way to
    // Editing's.
    void editingLayoutReplacesOldDefaultsAndKeepsOwnWorkspaces() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray before;
        QByteArray older;
        QByteArray docked;
        {
            MainWindow window{QString(), QString()};
            window.m_workspace->panel(QStringLiteral("presets"))->toggleView(false);
            before = window.m_workspace->manager()->saveState(5);
            older = window.m_workspace->manager()->saveState(3);
            // A layout of version 5 with the conversation docked as its own
            // panel (in place of the Inspector, in the browsers' column).
            docked = window.m_workspace->manager()->saveState(5);
            if (!docked.startsWith("<?xml")) docked = qUncompress(docked);
            QVERIFY(docked.contains("Name=\"inspector\""));
            docked.replace("Name=\"inspector\"", "Name=\"assistant\"");
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Review"), QStringLiteral("Old"), QStringLiteral("Docked")});
            settings.setValue(QStringLiteral("workspace/saved/Editing"), before);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), before);
            settings.setValue(QStringLiteral("workspace/saved/Review"), before);
            settings.setValue(QStringLiteral("workspace/saved/Old"), older);
            settings.setValue(QStringLiteral("workspace/saved/Docked"), docked);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        ads::CDockWidget *presets = workspace->panel(QStringLiteral("presets"));
        QVERIFY(!presets->isClosed());
        workspace->activate(QStringLiteral("Review"));
        QCOMPARE(workspace->current(), QStringLiteral("Review"));
        QVERIFY(presets->isClosed());
        QVERIFY(!window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        workspace->activate(QStringLiteral("Old"));
        QVERIFY(!presets->isClosed());
        workspace->activate(QStringLiteral("Docked"));
        QCOMPARE(workspace->current(), QStringLiteral("Docked"));
        QVERIFY(presets->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("program"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("assistant")));
        // The conversation is the stage's, whatever the workspace.
        QVERIFY(window.chatOpen());
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
            // The list's scroll bar shows only while the pointer is over the
            // panel: the panel's surface where its handle is, until then.
            {
                QScrollBar *listBar = window.m_chatScroll->verticalScrollBar();
                QVERIFY(listBar->isVisible());
                QCOMPARE(listBar->value(), listBar->maximum());
                const QPoint inHandle = listBar->mapTo(&window, QPoint(listBar->width() / 2, listBar->height() - 12));
                QEvent leave(QEvent::Leave);
                QCoreApplication::sendEvent(column, &leave);
                const QImage quiet = window.grab().toImage();
                QCOMPARE(quiet.pixelColor(inHandle * quiet.devicePixelRatio()).rgb(), editorTheme().chatSurface.rgb());
                QEnterEvent enter(QPointF(20, 20), QPointF(20, 20), column->mapToGlobal(QPointF(20, 20)));
                QCoreApplication::sendEvent(column, &enter);
                const QImage hovered = window.grab().toImage();
                QVERIFY(nearColor(hovered.pixelColor(inHandle * hovered.devicePixelRatio()).rgb(),
                                  laidOn(editorTheme().chatSurface, editorTheme().chatScrollHandle).rgb()));
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
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
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

    // The catalog's clip properties and generators, as a snapshot carries
    // them (crates/kerf-model catalog.rs), for the Inspector's tests.
    static QJsonObject catalogProperties() {
        const auto number = [](const char *name, double fallback, double min, double max, const char *unit = nullptr) {
            QJsonObject def{{"name", name}, {"type", "number"}, {"default", fallback}, {"min", min}, {"max", max}, {"animatable", true}};
            if (unit) def.insert("unit", unit);
            return def;
        };
        return QJsonObject{
            {"video", QJsonArray{
                number("x", 0, -10, 10, "frame widths"), number("y", 0, -10, 10, "frame heights"),
                number("scale", 1, 0, 100, "×"), number("scale_x", 1, -100, 100, "×"), number("scale_y", 1, -100, 100, "×"),
                number("rotation", 0, -36000, 36000, "degrees"), number("anchor_x", 0, -10, 10, "clip widths"),
                number("anchor_y", 0, -10, 10, "clip heights"), number("crop_l", 0, 0, 1, "fraction"), number("crop_r", 0, 0, 1, "fraction"),
                number("crop_t", 0, 0, 1, "fraction"), number("crop_b", 0, 0, 1, "fraction"), number("opacity", 1, 0, 1),
                QJsonObject{{"name", "blend"}, {"type", "enum"}, {"default", "normal"},
                            {"options", QJsonArray{"normal", "add", "multiply", "screen", "overlay", "darken", "lighten", "difference"}}}}},
            {"audio", QJsonArray{number("gain", 0, -96, 24, "dB"), number("pan", 0, -1, 1)}}};
    }
    static QJsonObject catalogGenerators() {
        return QJsonObject{
            {"solid", QJsonArray{QJsonObject{{"name", "color"}, {"type", "color"}, {"default", "#000000"}}}},
            {"title", QJsonArray{
                QJsonObject{{"name", "text"}, {"type", "text"}, {"default", ""}, {"doc", "The text to show."}},
                QJsonObject{{"name", "font"}, {"type", "text"}, {"default", "sans-serif"}},
                QJsonObject{{"name", "size"}, {"type", "number"}, {"default", 0.08}, {"min", 0.005}, {"max", 1.0}, {"unit", "frame heights"}},
                QJsonObject{{"name", "color"}, {"type", "color"}, {"default", "#ffffff"}},
                QJsonObject{{"name", "align"}, {"type", "enum"}, {"default", "center"}, {"options", QJsonArray{"center", "left", "right"}}}}},
            {"adjustment", QJsonArray{}}};
    }
    // The last edit's operations, taken.
    static QJsonArray takeOps(MainWindow &window) {
        const QJsonObject command = window.m_lastCommand;
        window.m_lastCommand = {};
        return command.value("type").toString() == QStringLiteral("edit") ? command.value("ops").toArray() : QJsonArray{};
    }

    // A slider never jumps on a stray click: a press away from its knob
    // changes nothing until the pointer moves, then moves the knob by as
    // much; a press on the knob takes it.
    void inspectorSliderIgnoresStrayClicks() {
        InspectorSlider slider;
        slider.resize(92, ScrubField::kHeight);
        slider.setRange(0.0, 100.0);
        slider.setOrigin(0.0);
        slider.setDetent(0.0);
        slider.setValue(50.0);
        QSignalSpy released(&slider, &InspectorSlider::released);
        const double knob = slider.knobCenter().x();
        // A click on the groove far from the knob: nothing.
        mouse(&slider, QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(10, 12));
        QCOMPARE(slider.value(), 50.0);
        QCOMPARE(released.size(), 0);
        // A drag from there: the knob moves by the pointer's travel.
        mouse(&slider, QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(&slider, QEvent::MouseMove, QPointF(20, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(20, 12));
        QCOMPARE(released.size(), 1);
        QVERIFY(std::abs(slider.knobCenter().x() - (knob + 10.0)) < 0.5);
        QCOMPARE(released.constLast().at(1).toDouble(), 50.0);
        // A press on the knob off its centre: the first move does not jump
        // the value by the offset; the knob moves by the pointer's travel.
        const double at = slider.knobCenter().x();
        const double grabbed = slider.value();
        mouse(&slider, QEvent::MouseButtonPress, QPointF(at + 5, 12));
        mouse(&slider, QEvent::MouseMove, QPointF(at + 4, 12));
        QVERIFY2(std::abs(slider.knobCenter().x() - (at - 1.0)) < 0.5, qPrintable(QString::number(slider.knobCenter().x() - at)));
        QVERIFY(slider.value() < grabbed);
        mouse(&slider, QEvent::MouseMove, QPointF(at - 20, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(at - 20, 12));
        QVERIFY(std::abs(slider.knobCenter().x() - (at - 25.0)) < 0.5);
        QCOMPARE(released.size(), 2);
        // The fill runs from the origin to the knob's centre, never past it;
        // at the origin there is none.
        InspectorSlider wide;
        wide.resize(84, ScrubField::kHeight);
        wide.setRange(-3.0, 3.0);
        wide.setOrigin(0.0);
        wide.setDetent(0.0);
        wide.setValue(0.0);
        QVERIFY(wide.fillRect().isEmpty());
        for (const double value : {0.1, 0.4, -0.2, 2.0}) {
            wide.setValue(value);
            const QRectF fill = wide.fillRect();
            const double knob = wide.knobCenter().x();
            if (value > 0) QVERIFY2(std::abs(fill.right() - knob) < 0.5, qPrintable(QString::number(value)));
            else QVERIFY2(std::abs(fill.left() - knob) < 0.5, qPrintable(QString::number(value)));
        }
    }

    // The default catches the knob on one pixel only: dragged slowly past
    // it on the Inspector's 84-point slider, every pixel gives a value of
    // its own, one step from the last, and exactly one gives the default.
    // ⌥ moves it a tenth as far; a double-click on the knob resets it.
    void inspectorSliderReachesValuesNearTheDefault() {
        const auto drag = [](InspectorSlider &slider, QEvent::Type type, double x, Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
            const QPointF at(x, ScrubField::kHeight / 2.0);
            const Qt::MouseButton button = type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton;
            const Qt::MouseButtons buttons = type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::MouseButtons(Qt::LeftButton);
            QMouseEvent event(type, at, slider.mapToGlobal(at), button, buttons, modifiers);
            QCoreApplication::sendEvent(&slider, &event);
        };
        struct Case {
            const char *what;
            double minimum, maximum, detent;
            bool gain;
        };
        // Saturation (0-400%, default 100%), Exposure (±3), Volume (0 dB).
        for (const Case &c : {Case{"saturation", 0.0, 4.0, 1.0, false}, Case{"exposure", -3.0, 3.0, 0.0, false},
                              Case{"volume", -96.0, 24.0, 0.0, true}}) {
            InspectorSlider slider;
            slider.resize(84, ScrubField::kHeight);
            slider.setRange(c.minimum, c.maximum);
            slider.setGainTravel(c.gain);
            slider.setOrigin(c.detent);
            slider.setDetent(c.detent);
            slider.setValue(c.detent);
            const double knob = slider.knobCenter().x();
            const int from = int(std::floor(knob)) - 8;
            const int to = int(std::floor(knob)) + 8;
            drag(slider, QEvent::MouseButtonPress, std::floor(knob));
            QList<double> values;
            for (int x = from; x <= to; ++x) {
                drag(slider, QEvent::MouseMove, x);
                values << slider.value();
            }
            drag(slider, QEvent::MouseButtonRelease, to);
            int atDefault = 0;
            for (int i = 0; i < values.size(); ++i) {
                if (values.at(i) == c.detent) ++atDefault;
                if (i == 0) continue;
                // Each pixel its own value, in order: none hidden behind the default.
                QVERIFY2(values.at(i) > values.at(i - 1), qPrintable(QStringLiteral("%1 at %2: %3 after %4").arg(c.what).arg(from + i)
                                                                        .arg(values.at(i)).arg(values.at(i - 1))));
                // No jump: the travel between two pixels is at most a pixel
                // and a half's (the caught pixel is at most half a pixel off).
                const double step = slider.travelOf(values.at(i)) - slider.travelOf(values.at(i - 1));
                const double pixel = 1.0 / slider.groove().width();
                QVERIFY2(step <= 1.5 * pixel + 1e-9, qPrintable(QStringLiteral("%1 at %2: %3 px").arg(c.what).arg(from + i).arg(step / pixel)));
            }
            QVERIFY2(atDefault == 1, qPrintable(QStringLiteral("%1: %2 pixels give the default").arg(c.what).arg(atDefault)));
        }
        InspectorSlider slider;
        slider.resize(84, ScrubField::kHeight);
        slider.setRange(0.0, 4.0);
        slider.setOrigin(0.0);
        slider.setDetent(1.0);
        slider.setValue(1.0);
        QSignalSpy released(&slider, &InspectorSlider::released);
        const double knob = slider.knobCenter().x();
        const double pixel = 4.0 / slider.groove().width();
        // 3 points left of the default: about 0.83, not the default.
        drag(slider, QEvent::MouseButtonPress, knob);
        drag(slider, QEvent::MouseMove, knob - 3.0);
        drag(slider, QEvent::MouseButtonRelease, knob - 3.0);
        QVERIFY2(std::abs(slider.value() - (1.0 - 3.0 * pixel)) < 1e-9, qPrintable(QString::number(slider.value())));
        QCOMPARE(released.size(), 1);
        // With ⌥ held, 20 points of travel move it 2.
        slider.setValue(1.0);
        drag(slider, QEvent::MouseButtonPress, knob, Qt::AltModifier);
        drag(slider, QEvent::MouseMove, knob - 20.0, Qt::AltModifier);
        drag(slider, QEvent::MouseButtonRelease, knob - 20.0, Qt::AltModifier);
        QVERIFY2(std::abs(slider.value() - (1.0 - 2.0 * pixel)) < 1e-9, qPrintable(QString::number(slider.value())));
        // A double-click on the knob: back to the default, one edit.
        const int before = int(released.size());
        const double changed = slider.value();
        drag(slider, QEvent::MouseButtonPress, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonRelease, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonDblClick, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonRelease, slider.knobCenter().x());
        QCOMPARE(slider.value(), 1.0);
        QCOMPARE(int(released.size()), before + 1);
        QCOMPARE(released.constLast().at(0).toDouble(), 1.0);
        QCOMPARE(released.constLast().at(1).toDouble(), changed);
    }

    // A slider's default stays in sight in both appearances without
    // anything drawn on the knob: under the knob the 1pt tick's ends show
    // above and below it, crisp, in the detent ink; the knob's inside is
    // its own colour only. A changed value's knob edge stands out against
    // the groove and panel; at the default it is quieter.
    void inspectorSliderDefaultShowsAroundTheKnob() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); applyEditorAppearance(); });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            applyEditorAppearance();
            const EditorTheme &theme = editorTheme();
            // The knob's edge is read against what lies round it: a changed
            // knob's is the more visible one against both the groove and
            // the panel, at least 3:1 on the groove, in both appearances.
            const QString look = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");
            QVERIFY(theme.sliderHandleChanged != theme.sliderHandleAtDefault);
            for (const QColor &around : {theme.sliderGroove, theme.panel})
                QVERIFY2(contrastRatio(theme.sliderHandleChanged, around) > contrastRatio(theme.sliderHandleAtDefault, around),
                         qPrintable(look + QStringLiteral(" on ") + around.name()));
            QVERIFY2(contrastRatio(theme.sliderHandleChanged, theme.sliderGroove) >= 3.0, qPrintable(look));
            QVERIFY2(contrastRatio(theme.sliderHandleChanged, theme.panel) >= 3.0, qPrintable(look));
            InspectorSlider slider;
            slider.resize(92, ScrubField::kHeight);
            slider.setRange(-3.0, 3.0);
            slider.setOrigin(0.0);
            slider.setDetent(0.0);
            for (const double value : {0.0, 0.1}) {
                slider.setValue(value);
                const QString what = QStringLiteral("%1 %2").arg(theme.dark ? "dark" : "light").arg(value);
                QCOMPARE(slider.knobEdge(), value == 0.0 ? theme.sliderHandleAtDefault : theme.sliderHandleChanged);
                const QRect tick = slider.detentTick();
                const QPointF knob = slider.knobCenter();
                QCOMPARE(tick.width(), 1);
                QVERIFY2(tick.top() <= knob.y() - InspectorSlider::kKnobRadius - 3.0, qPrintable(what));
                QVERIFY2(tick.bottom() + 1 >= knob.y() + InspectorSlider::kKnobRadius + 3.0, qPrintable(what));
                QImage image(slider.size(), QImage::Format_ARGB32_Premultiplied);
                image.fill(theme.panel);
                slider.render(&image);
                // The tick's ends: the detent ink, solid, above and below.
                for (const int y : {tick.top(), tick.top() + 2, tick.bottom() - 2, tick.bottom()})
                    QVERIFY2(image.pixelColor(tick.left(), y) == theme.inspectorDetent, qPrintable(what + QStringLiteral(" y %1").arg(y)));
                // Inside the knob (clear of its 1pt edge): the knob's colour only.
                for (int y = 0; y < image.height(); ++y)
                    for (int x = 0; x < image.width(); ++x) {
                        const QPointF centre(x + 0.5, y + 0.5);
                        if (QLineF(centre, knob).length() > InspectorSlider::kKnobRadius - 1.5) continue;
                        QVERIFY2(image.pixelColor(x, y) == theme.sliderHandle,
                                 qPrintable(what + QStringLiteral(" at %1,%2").arg(x).arg(y)));
                    }
            }
            // Away from the knob, the plain 1x10 mark.
            slider.setValue(2.0);
            QCOMPARE(slider.detentTick().height(), 10);
        }
    }

    // A switched-off effect's values stay editable and readable: numbers,
    // slider fills and the colour's hex in the secondary ink (4.5:1 on the
    // field), never the disabled one.
    void switchedOffEffectValuesStayReadable() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}, {"effects", QJsonArray{
                                    QJsonObject{{"fx", "light.edgeglow"}, {"enabled", false}, {"params", QJsonObject{{"radius", 40}, {"intensity", 200}}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        QWidget *box = inspector->findChild<QWidget *>(QStringLiteral("effectBox"));
        QVERIFY(box);
        QVERIFY(!box->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"))->isChecked());
        const EditorTheme &theme = editorTheme();
        QVERIFY(contrastRatio(theme.textSecondary, theme.inspectorField) >= 4.5);
        const auto fields = box->findChildren<ScrubField *>();
        QVERIFY(!fields.isEmpty());
        for (ScrubField *field : fields) {
            QVERIFY(field->isDimmed());
            QCOMPARE(field->valueInk(), theme.textSecondary);
        }
        const auto sliders = box->findChildren<InspectorSlider *>();
        QVERIFY(!sliders.isEmpty());
        for (InspectorSlider *slider : sliders) {
            QVERIFY(slider->isDimmed() && slider->isEnabled());
            QCOMPARE(slider->fillInk(), theme.textSecondary);
        }
        for (QLabel *hex : box->findChildren<QLabel *>(QStringLiteral("colorHex"))) {
            QVERIFY(hex->property("off").toBool());
            QCOMPARE(hex->palette().color(QPalette::WindowText), theme.textSecondary);
        }
    }

    // Each effect is a header with a switch, its name and ⋯ (Reset, Ask
    // Kerf, Remove), no ✕; folded, a second line lists what changed; its
    // parameters are scrub fields with units, five before "N more", grouped
    // under quiet sub-headers when the catalog groups them.
    void inspectorShowsAndEditsTheEffectStack() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}, {"effects", QJsonArray{
                                    QJsonObject{{"fx", "light.edgeglow"}, {"enabled", true}, {"params", QJsonObject{{"radius", 40}, {"intensity", 200}}}},
                                    QJsonObject{{"fx", "channel.invert"}, {"enabled", false}, {"params", QJsonObject{}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        InspectorSection *effects = inspector->section(QStringLiteral("effects"));
        QVERIFY(effects);
        // Folded, its header names every effect it counts, one switched
        // off marked so.
        QVERIFY(!effects->isOpen());
        QCOMPARE(effects->summary(), QStringLiteral("Edge Glow, Invert (off)"));
        // Shown in whole names only: the ones on, then the count switched
        // off; short of room, "1 on · 1 off"; shorter still, nothing.
        {
            QFont font;
            font.setPixelSize(12);
            const QFontMetrics metrics(font);
            InspectorSection::Summary summary{{QStringLiteral("Primary Colour Correction"), QStringLiteral("Gaussian Blur")},
                                              QStringLiteral(" · 1 off"), QStringLiteral("2 on · 1 off"), QString()};
            const auto fit = [&](const QString &text) { return InspectorSection::fitSummary(summary, metrics, metrics.horizontalAdvance(text)); };
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction, Gaussian Blur · 1 off")), QStringLiteral("Primary Colour Correction, Gaussian Blur · 1 off"));
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction +1 · 1 off")), QStringLiteral("Primary Colour Correction +1 · 1 off"));
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction")), QStringLiteral("2 on · 1 off"));
            QCOMPARE(InspectorSection::fitSummary(summary, metrics, 8), QString());
            QCOMPARE(effects->shownSummary().contains(QChar(0x2026)), false);
        }
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 2);
        auto *glow = boxes.at(0)->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"));
        auto *radius = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("effectParam_radius"));
        auto *intensity = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("effectParam_intensity"));
        QVERIFY(glow && radius && intensity);
        QCOMPARE(boxes.at(0)->findChild<QLabel *>(QStringLiteral("effectName"))->text(), QStringLiteral("Edge Glow"));
        QVERIFY(glow->isChecked());
        QCOMPARE(radius->value(), 40.0);
        // "Intensity 150.000" reads "200%" now, with a slider.
        QCOMPARE(intensity->text(), QStringLiteral("200"));
        QCOMPARE(intensity->format().unit, QStringLiteral("%"));
        QVERIFY(boxes.at(0)->findChild<InspectorSlider *>(QStringLiteral("slider_intensity")));
        QVERIFY(!boxes.at(1)->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"))->isChecked());
        QVERIFY(boxes.at(1)->findChild<QLabel *>(QStringLiteral("effectName"))->property("off").toBool());
        // Folded, the second line lists the values that differ.
        auto *summary = boxes.at(0)->findChild<QLabel *>(QStringLiteral("effectSummary"));
        QCOMPARE(summary->text(), QStringLiteral("Glow radius 40 px · Intensity 200%"));
        QVERIFY(!summary->isHidden());
        QVERIFY(boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        // No ✕ anywhere.
        for (QAbstractButton *button : inspector->findChildren<QAbstractButton *>()) {
            QVERIFY(button->text() != QStringLiteral("✕"));
            QVERIFY(button->objectName() != QStringLiteral("effectRemove"));
        }
        // Opening it shows its rows and hides the line; it stays open for the effect.
        boxes.at(0)->findChild<GlyphButton *>(QStringLiteral("effectChevron"))->click();
        QVERIFY(!boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        QVERIFY(summary->isHidden());
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/fx/light.edgeglow/open")).toBool());
        // Row labels keep one 12-point line: a first word the effect's name
        // says goes ("Edge Threshold" under Edge Glow reads "Threshold"),
        // the catalog's label whole in the tooltip; "Glow only" keeps its
        // first word, since "Only" alone says nothing.
        {
            const auto labelOf = [&](const char *param) -> RowLabel * {
                for (QWidget *at = boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParam_%1").arg(QLatin1String(param))); at; at = at->parentWidget())
                    if (auto *row = qobject_cast<InspectorRow *>(at)) return row->label();
                return nullptr;
            };
            RowLabel *threshold = labelOf("threshold");
            QVERIFY(threshold);
            QCOMPARE(threshold->text(), QStringLiteral("Threshold"));
            QCOMPARE(threshold->toolTip(), QStringLiteral("Edge Threshold"));
            QCOMPARE(labelOf("width")->text(), QStringLiteral("Width"));
            QCOMPARE(labelOf("radius")->text(), QStringLiteral("Radius"));
            QCOMPARE(labelOf("color")->text(), QStringLiteral("Colour"));
            QCOMPARE(labelOf("color")->toolTip(), QStringLiteral("Glow Colour"));
            QCOMPARE(labelOf("only")->text(), QStringLiteral("Glow only"));
            for (RowLabel *label : boxes.at(0)->findChildren<RowLabel *>()) {
                QCOMPARE(label->font().pixelSize(), 12);
                QVERIFY(!label->wordWrap() && !label->text().contains(QLatin1Char('\n')));
                QVERIFY2(label->sizeHint().height() <= QFontMetrics(label->font()).lineSpacing() + 2, qPrintable(label->text()));
            }
            // Too long for the column even so: one line, ended in an ellipsis.
            RowLabel lengthy(QStringLiteral("Highlight threshold of the brightest spots"));
            lengthy.resize(InspectorRow::kLabelWidth, 20);
            lengthy.show();
            QCoreApplication::processEvents();
            QVERIFY(lengthy.text().endsWith(QChar(0x2026)) && !lengthy.text().contains(QLatin1Char('\n')));
            QCOMPARE(lengthy.toolTip(), QStringLiteral("Highlight threshold of the brightest spots"));
            QCOMPARE(lengthy.font().pixelSize(), 12);
        }
        // Names and labels read as the rest of the UI: effects in Title
        // Case, labels in sentence case, "colour" spelt as elsewhere.
        QCOMPARE(presentationEffectName(QStringLiteral("Primary color correction.")), QStringLiteral("Primary Colour Correction"));
        QCOMPARE(presentationEffectName(QStringLiteral("Gaussian blur.")), QStringLiteral("Gaussian Blur"));
        QCOMPARE(presentationEffectName(QStringLiteral("Edge Glow")), QStringLiteral("Edge Glow"));
        QCOMPARE(sentenceCase(QStringLiteral("Look LUT")), QStringLiteral("Look LUT"));
        QCOMPARE(sentenceCase(QStringLiteral("Glow Color")), QStringLiteral("Glow colour"));
        // Edge Glow has six parameters: folding one would save nothing, so
        // all six show and there is no "1 more".
        {
            QVERIFY(!boxes.at(0)->findChild<QPushButton *>(QStringLiteral("effectMoreToggle")));
            QWidget *moreRows = boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectMoreRows"));
            int params = 0;
            for (QWidget *child : boxes.at(0)->findChildren<QWidget *>())
                if (child->objectName().startsWith(QStringLiteral("effectParam_"))) {
                    ++params;
                    QVERIFY2(!moreRows || !moreRows->isAncestorOf(child), qPrintable(child->objectName()));
                }
            QCOMPARE(params, 6);
        }
        auto op = [&window] { return takeOps(window).first().toObject(); };
        glow->click();
        QJsonObject edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(edit.value("index").toInt(), 0);
        QCOMPARE(edit.value("enabled").toBool(), false);
        radius->commitText(QStringLiteral("10")); // the default
        edit = op();
        QVERIFY(edit.value("params").toObject().contains("radius"));
        QVERIFY(edit.value("params").toObject().value("radius").isNull());
        // ⋯ ▸ Remove and ⋯ ▸ Reset Parameters.
        boxes.at(1)->findChild<QAction *>(QStringLiteral("effectRemove"))->trigger();
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-remove"));
        QCOMPARE(edit.value("index").toInt(), 1);
        boxes.at(0)->findChild<QAction *>(QStringLiteral("effectReset"))->trigger();
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-set"));
        QVERIFY(edit.value("params").toObject().value("radius").isNull());
        QVERIFY(edit.value("params").toObject().value("intensity").isNull());
        // Delete on a focused effect header removes it.
        key(boxes.at(1)->findChild<QWidget *>(QStringLiteral("effectHeader")), Qt::Key_Delete);
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-remove"));
        QCOMPARE(edit.value("index").toInt(), 1);
        // ⋯ ▸ Ask Kerf about This Effect.
        QSignalSpy asked(inspector, &InspectorPanel::askRequested);
        boxes.at(0)->findChild<QAction *>(QStringLiteral("effectAsk"))->trigger();
        QCOMPARE(asked.size(), 1);
        QVERIFY(asked.first().first().toString().endsWith(QStringLiteral("Edge Glow")));

        // Grouped parameters ("Vignette: Amount") sit under sub-headers, and
        // "N more" counts the groups after the first.
        inspector->m_effectDefs.insert(QStringLiteral("test.grouped"), QJsonObject{{"id", "test.grouped"}, {"name", "Grouped"}, {"params", QJsonArray{
            QJsonObject{{"name", "temperature"}, {"type", "number"}, {"default", 0}, {"min", -100}, {"max", 100}, {"doc", "Basic Correction: Temperature"}},
            QJsonObject{{"name", "tint"}, {"type", "number"}, {"default", 0}, {"min", -100}, {"max", 100}, {"doc", "Basic Correction: Tint"}},
            QJsonObject{{"name", "amount"}, {"type", "number"}, {"default", 0}, {"min", -5}, {"max", 5}, {"doc", "Vignette: Amount"}},
            QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 50}, {"min", 0}, {"max", 100}, {"doc", "Vignette: Feather"}},
            QJsonObject{{"name", "grain"}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 100}, {"doc", "Film: Grain"}}}}});
        inspector->m_shape.clear();
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                            {"effects", QJsonArray{QJsonObject{{"fx", "test.grouped"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        QWidget *grouped = inspector->findChildren<QWidget *>(QStringLiteral("effectBox")).value(0);
        QVERIFY(grouped);
        QStringList groups;
        for (QLabel *label : grouped->findChildren<QLabel *>(QStringLiteral("effectGroup"))) groups << label->text();
        QCOMPARE(groups, (QStringList{"Basic Correction", "Vignette", "Film"}));
        QCOMPARE(grouped->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"))->text(), QStringLiteral("2 more"));
        QWidget *amount = grouped->findChild<ScrubField *>(QStringLiteral("effectParam_amount"));
        QVERIFY(amount && grouped->findChild<QWidget *>(QStringLiteral("effectMoreRows"))->isAncestorOf(amount));
        auto *tintRow = qobject_cast<InspectorRow *>(grouped->findChild<ScrubField *>(QStringLiteral("effectParam_tint"))->parentWidget());
        QVERIFY(tintRow);
        QCOMPARE(tintRow->label()->toolTip(), QStringLiteral("Basic Correction: Tint"));

        // Seven parameters: five, then "2 more ⌄", in words as Add effect
        // is, at the column's start; open stays open per effect.
        QJsonArray seven;
        for (int index = 0; index < 7; ++index)
            seven.append(QJsonObject{{"name", QStringLiteral("p%1").arg(index)}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 100},
                                     {"doc", QStringLiteral("Amount %1").arg(index)}});
        inspector->m_effectDefs.insert(QStringLiteral("test.seven"), QJsonObject{{"id", "test.seven"}, {"name", "Seven"}, {"params", seven}});
        inspector->m_shape.clear();
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                            {"effects", QJsonArray{QJsonObject{{"fx", "test.seven"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        QWidget *long7 = inspector->findChildren<QWidget *>(QStringLiteral("effectBox")).value(0);
        QVERIFY(long7);
        auto *more = long7->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"));
        QVERIFY(more);
        QCOMPARE(more->text(), QStringLiteral("2 more"));
        QCOMPARE(more->font().pixelSize(), 12);
        QCOMPARE(more->mapTo(long7, QPoint()).x(), 0);
        QWidget *moreRows = long7->findChild<QWidget *>(QStringLiteral("effectMoreRows"));
        QVERIFY(moreRows->isHidden());
        QVERIFY(moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p5"))));
        QVERIFY(!moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p4"))));
        more->click();
        QVERIFY(!moreRows->isHidden());
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/fx/test.seven/more")).toBool());
    }

    // Add effect raises the Effects browser with its search ready; adding
    // one brings the Inspector back with it open. Dropping an effect on the
    // Inspector inserts it where it lands.
    void inspectorAddsAndDropsEffects() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, int effects) {
            QJsonArray stack;
            for (int index = 0; index < effects; ++index)
                stack.append(QJsonObject{{"fx", index % 2 ? "blur.gaussian" : "light.edgeglow"}, {"enabled", true}, {"params", QJsonObject{}}});
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"effects", stack}}}}}}}}}});
        };
        snapshot(1, 2);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("c1"));
        Workspace *workspace = window.m_workspace;
        QVERIFY(workspace->panel(QStringLiteral("inspector"))->isCurrentTab());
        InspectorPanel *inspector = window.m_inspector;
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        QCoreApplication::processEvents();
        auto *add = inspector->findChild<QPushButton *>(QStringLiteral("addEffect"));
        QVERIFY(add && add->isVisible());
        QCOMPARE(add->text(), QStringLiteral("Add effect"));
        add->click();
        QVERIFY(workspace->panel(QStringLiteral("effects"))->isCurrentTab());
        QVERIFY(window.m_effects->m_search->hasFocus() || QApplication::focusWidget() == window.m_effects->m_search);
        // Adding: one fx-add, and the Inspector is back.
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        const QJsonObject added = takeOps(window).first().toObject();
        QCOMPARE(added.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(added.value("fx").toString(), QStringLiteral("blur.gaussian"));
        QVERIFY(workspace->panel(QStringLiteral("inspector"))->isCurrentTab());
        // The snapshot with it shows it open.
        snapshot(2, 3);
        QCoreApplication::processEvents();
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 3);
        QVERIFY(!boxes.at(2)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        // The section's + does what Add effect does.
        QSignalSpy wanted(inspector, &InspectorPanel::addEffectRequested);
        inspector->findChild<GlyphButton *>(QStringLiteral("effectsAdd"))->click();
        QCOMPARE(wanted.size(), 1);
        QCOMPARE(wanted.first().first().toBool(), false);
        // A drop between the first and second effects inserts at 1.
        workspace->panel(QStringLiteral("inspector"))->setAsCurrentTab();
        QCoreApplication::processEvents();
        const QWidget *second = boxes.at(1);
        const QPoint at = second->mapTo(inspector, QPoint(20, 4));
        QMimeData data;
        data.setData(QStringLiteral("application/x-kerf-fx"), QByteArray("color.basic"));
        QDragEnterEvent enter(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &enter);
        QVERIFY(enter.isAccepted());
        QDragMoveEvent move(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &move);
        QCOMPARE(inspector->m_dropIndex, 1);
        QVERIFY(inspector->m_dropLine->isVisible());
        QCOMPARE(inspector->m_dropLine->height(), 2);
        QDropEvent drop(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &drop);
        const QJsonObject dropped = takeOps(window).first().toObject();
        QCOMPARE(dropped.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(dropped.value("fx").toString(), QStringLiteral("color.basic"));
        QCOMPARE(dropped.value("index").toInt(), 1);
        QVERIFY(!inspector->m_dropLine->isVisible());
        // The Effects browser's cards drag as their effect.
        QListWidgetItem *card = window.m_effects->m_items.value(QStringLiteral("blur.gaussian"));
        QVERIFY(card);
        std::unique_ptr<QMimeData> dragged(window.m_effects->m_cards->model()->mimeData({window.m_effects->m_cards->model()->index(window.m_effects->m_cards->row(card), 0)}));
        QVERIFY(dragged);
        QCOMPARE(dragged->data(QStringLiteral("application/x-kerf-fx")), QByteArray("blur.gaussian"));
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
        QVERIFY(!window.m_inspector->section(QStringLiteral("masks")));
        window.selectClip(QStringLiteral("c1"));
        QVERIFY(rectangle->isEnabled());
        // The Inspector's Masks section, with the tools in its header.
        InspectorSection *masksSection = window.m_inspector->section(QStringLiteral("masks"));
        QVERIFY(masksSection);
        QCOMPARE(masksSection->header()->findChildren<QToolButton *>(QStringLiteral("maskTool")).size(), 3);
        QVERIFY(window.m_inspector->findChild<QLabel *>(QStringLiteral("masksHint")));
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

        // The Inspector's Masks section edits its mode, inversion, and
        // properties (feather in px), and an effect's mask parameter picks it.
        snapshot(5, QJsonArray{k1});
        InspectorPanel *inspector = window.m_inspector;
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
        QCOMPARE(boxes.size(), 1);
        QVERIFY(boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->isChecked());
        // The selected mask is open.
        QVERIFY(!boxes.at(0)->findChild<QWidget *>(QStringLiteral("maskBody"))->isHidden());
        QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("1"));
        auto *mode = boxes.at(0)->findChild<QComboBox *>(QStringLiteral("maskMode"));
        auto *inverted = boxes.at(0)->findChild<InspectorSwitch *>(QStringLiteral("maskInverted"));
        auto *feather = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("maskParam_feather"));
        auto *maskOpacity = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("maskParam_opacity"));
        QVERIFY(mode && inverted && feather && maskOpacity);
        QCOMPARE(feather->format().unit, QStringLiteral("px"));
        QCOMPARE(maskOpacity->format().unit, QStringLiteral("%"));
        QCOMPARE(maskOpacity->text(), QStringLiteral("100"));
        QCOMPARE(mode->count(), 3);
        QCOMPARE(mode->currentText(), QStringLiteral("add"));
        mode->setCurrentIndex(2);
        emit mode->activated(2);
        set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("mode").toString(), QStringLiteral("none"));
        inverted->click();
        QCOMPARE(op().value("inverted").toBool(), true);
        feather->commitText(QStringLiteral("12"));
        set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("name").toString(), QStringLiteral("k1"));
        QCOMPARE(set.value("params").toObject().value("feather").toDouble(), 12.0);
        snapshot(5, QJsonArray{QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "add"},
                                           {"inverted", false}, {"params", QJsonObject{{"feather", 12.0}}}}});
        QCOMPARE(feather->text(), QStringLiteral("12"));
        feather->commitText(QStringLiteral("0"));
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
        // removes it, as ⋯ ▸ Remove in the panel does.
        viewer->selectMask(QString());
        boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->click();
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        key(viewer, Qt::Key_Delete);
        QJsonObject removed = op();
        QCOMPARE(removed.value("op").toString(), QStringLiteral("mask-remove"));
        QCOMPARE(removed.value("name").toString(), QStringLiteral("k1"));
        snapshot(7, QJsonArray{k1});
        inspector->findChild<QAction *>(QStringLiteral("maskRemove"))->trigger();
        QCOMPARE(op().value("op").toString(), QStringLiteral("mask-remove"));
        // Selecting it on the picture opens its row there.
        snapshot(7, QJsonArray{k1});
        QWidget *body = inspector->findChild<QWidget *>(QStringLiteral("maskBody"));
        inspector->m_openMasks.clear();
        body->hide();
        viewer->selectMask(QStringLiteral("k1"));
        emit viewer->maskSelected(QStringLiteral("k1"));
        QVERIFY(!body->isHidden());

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
        // Two masks are two rows of the section, a row's height apart; the
        // section's title and the masks' names are semibold.
        {
            QJsonObject k2 = k1;
            k2.insert("name", "k2");
            snapshot(9, QJsonArray{k1, k2});
            const auto pair = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
            QCOMPARE(pair.size(), 2);
            inspector->ensurePolished();
            QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("2"));
            QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionTitle"))->font().weight(), QFont::DemiBold);
            for (QWidget *box : pair) {
                box->ensurePolished();
                auto *name = box->findChild<QToolButton *>(QStringLiteral("maskName"));
                name->ensurePolished();
                QCOMPARE(name->font().weight(), QFont::DemiBold);
            }
        }
        // Deselecting the clip hides its masks and stops a tool.
        rectangle->trigger();
        window.selectClip(QString());
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(viewer->m_maskClip.isEmpty());
        QVERIFY(!rectangle->isEnabled());
    }

    // Inspector rows for the catalog's properties of the clip's track type,
    // with units: position in px of the frame, scale and opacity in %, gain
    // in dB, pan L…R; a title's text, font, size and colour; an effect's
    // file, track and sound parameters. An animated value shows its value
    // at the playhead, read-only, with ◆ and no keyframe button.
    void inspectorEditsTheCatalogPropertiesOfTheTrackType() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_inspector->m_effectDefs.insert(QStringLiteral("test.params"), QJsonObject{{"id", "test.params"}, {"name", "Params"}, {"params", QJsonArray{
            QJsonObject{{"name", "map"}, {"type", "track"}, {"default", ""}},
            QJsonObject{{"name", "audioLayer"}, {"type", "sound"}, {"default", ""}},
            QJsonObject{{"name", "lut"}, {"type", "file"}, {"extensions", QJsonArray{"cube"}}, {"default", ""}, {"doc", "LUT"}}}}});
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()}, {"generators", catalogGenerators()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"opacity", 0.5}, {"x", QJsonArray{
                                    QJsonObject{{"at", 0.0}, {"value", 0.25}, {"interp", "linear"}},
                                    QJsonObject{{"at", 1.0}, {"value", 1.0}, {"interp", "linear"}}}}}}}}}},
                QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "g"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"text", "Day One"}}}}}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", QJsonObject{{"gain", -6.0}, {"pan", -0.4}}},
                                {"effects", QJsonArray{QJsonObject{{"fx", "test.params"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        auto set = [&window] {
            const QJsonArray ops = takeOps(window);
            const QJsonObject op = ops.first().toObject();
            return op.value("op").toString() == QStringLiteral("set") ? op : QJsonObject{};
        };
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->findChildren<ScrubField *>(QStringLiteral("prop_opacity")).isEmpty());
        window.selectClip(QStringLiteral("v"));
        const auto field = [inspector](const char *name) { return inspector->findChild<ScrubField *>(QString::fromLatin1(name)); };
        ScrubField *opacity = field("prop_opacity");
        ScrubField *x = field("prop_x");
        ScrubField *scale = field("prop_scale");
        ScrubField *rotation = field("prop_rotation");
        auto *blend = inspector->findChild<QComboBox *>(QStringLiteral("prop_blend"));
        QVERIFY(opacity && x && scale && rotation && blend);
        // Units: px of the 1080-wide frame, %, °.
        QCOMPARE(opacity->text(), QStringLiteral("50"));
        QCOMPARE(opacity->format().unit, QStringLiteral("%"));
        QCOMPARE(scale->format().unit, QStringLiteral("%"));
        QCOMPARE(scale->text(), QStringLiteral("100"));
        QCOMPARE(rotation->format().unit, QStringLiteral("°"));
        QCOMPARE(x->format().unit, QStringLiteral("px"));
        QCOMPARE(x->format().prefix, QStringLiteral("X"));
        // Animated: its value at the playhead (0.25 of 1080), read-only, ◆.
        auto *xRow = qobject_cast<InspectorRow *>(x->parentWidget());
        QVERIFY(xRow);
        QVERIFY(x->isReadOnly());
        QCOMPARE(x->text(), QStringLiteral("270"));
        QVERIFY(xRow->isAnimated());
        QVERIFY(xRow->findChild<QLabel *>(QStringLiteral("animatedMark"))->toolTip().contains(QStringLiteral("2 keyframes")));
        inspector->setPlayhead(0.5);
        QCOMPARE(x->text(), QStringLiteral("675"));
        for (QWidget *widget : inspector->findChildren<QWidget *>())
            QVERIFY(!widget->objectName().contains(QStringLiteral("keyframe"), Qt::CaseInsensitive));
        QCOMPARE(blend->currentText(), QStringLiteral("Normal"));
        opacity->commitText(QStringLiteral("50%"));
        QVERIFY(takeOps(window).isEmpty());
        opacity->commitText(QStringLiteral("75"));
        QCOMPARE(set().value("props").toObject().value("opacity").toDouble(), 0.75);
        opacity->commitText(QStringLiteral("100%"));
        QVERIFY(set().value("props").toObject().value("opacity").isNull());
        blend->setCurrentIndex(1);
        emit blend->activated(1);
        QCOMPARE(set().value("props").toObject().value("blend").toString(), QStringLiteral("add"));

        // A title: its text, font, size in px of the frame's height, colour.
        window.selectClip(QStringLiteral("g"));
        auto *text = inspector->findChild<QPlainTextEdit *>(QStringLiteral("titleText"));
        auto *color = inspector->findChild<QPushButton *>(QStringLiteral("titleColor"));
        auto *font = inspector->findChild<QComboBox *>(QStringLiteral("titleFont"));
        ScrubField *size = field("titleSize");
        QVERIFY(text && color && font && size);
        QCOMPARE(text->toPlainText(), QStringLiteral("Day One"));
        // The generic family by the family it resolves to; the model keeps "sans-serif".
        QCOMPARE(font->currentData().toString(), QStringLiteral("sans-serif"));
        QVERIFY(font->currentText() != QStringLiteral("sans-serif") && !font->currentText().isEmpty());
        QCOMPARE(size->text(), QStringLiteral("154"));
        QCOMPARE(size->format().unit, QStringLiteral("px"));
        // A colour well: the swatch, with the value beside it as hex.
        QCOMPARE(color->property("modelColor").toString(), QStringLiteral("#ffffff"));
        QVERIFY(color->text().isEmpty());
        QVERIFY(!color->icon().isNull());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("colorHex"))->text(), QStringLiteral("#FFFFFF"));
        QCOMPARE(modelColor(QStringLiteral("#ff000080")).alpha(), 0x80);
        QCOMPARE(modelColorName(QColor(255, 0, 0, 0x80)), QStringLiteral("#ff000080"));
        QCOMPARE(modelColorName(Qt::white), QStringLiteral("#ffffff"));
        // The text commits when it loses focus.
        text->setPlainText(QStringLiteral("Day Two"));
        QFocusEvent out(QEvent::FocusOut, Qt::OtherFocusReason);
        QCoreApplication::sendEvent(text, &out);
        QCOMPARE(set().value("props").toObject().value("text").toString(), QStringLiteral("Day Two"));
        const QJsonObject colorDef = catalogGenerators().value("title").toArray().at(3).toObject();
        inspector->commitValue(InspectorPanel::Target{InspectorPanel::Target::Kind::Prop, QStringLiteral("color")}, colorDef, QStringLiteral("#ffffff"));
        QVERIFY(takeOps(window).isEmpty());
        inspector->commitValue(InspectorPanel::Target{InspectorPanel::Target::Kind::Prop, QStringLiteral("color")}, colorDef, QStringLiteral("#ff0000"));
        QCOMPARE(set().value("props").toObject().value("color").toString(), QStringLiteral("#ff0000"));

        // A sound clip: gain in dB (−∞ at the floor), pan from L to R.
        window.selectClip(QStringLiteral("a"));
        ScrubField *gain = field("prop_gain");
        ScrubField *pan = field("prop_pan");
        QVERIFY(gain && pan);
        QCOMPARE(gain->format().unit, QStringLiteral("dB"));
        QCOMPARE(gain->text(), QString(QChar(0x2212)) + QStringLiteral("6.0"));
        QCOMPARE(pan->text(), QStringLiteral("L 40"));
        QVERIFY(inspector->findChild<InspectorSlider *>(QStringLiteral("slider_gain")));
        gain->commitText(QStringLiteral("-3dB"));
        QCOMPARE(set().value("props").toObject().value("gain").toDouble(), -3.0);
        gain->commitText(QStringLiteral("-inf"));
        QCOMPARE(set().value("props").toObject().value("gain").toDouble(), -96.0);
        QCOMPARE(gain->text(), QString(QChar(0x2212)) + QChar(0x221e));
        pan->commitText(QStringLiteral("Centre"));
        QVERIFY(set().value("props").toObject().value("pan").isNull());
        QCOMPARE(pan->text(), QStringLiteral("Centre"));
        // A file parameter is a field with a folder glyph inside its right
        // end that chooses the file; no push button beside it.
        inspector->findChild<QWidget *>(QStringLiteral("effectParams"))->show();
        QWidget *file = inspector->findChild<QWidget *>(QStringLiteral("effectParam_lut"));
        QVERIFY(file);
        auto *fileLine = file->findChild<QLineEdit *>();
        QVERIFY(fileLine && !file->findChild<QPushButton *>());
        auto *fileChoose = fileLine->findChild<QAction *>(QStringLiteral("chooseFile"));
        QVERIFY(fileChoose && !fileChoose->icon().isNull());
        QCOMPARE(fileChoose->toolTip(), QStringLiteral("Choose a file"));
        file->resize(240, 22);
        file->layout()->activate();
        QCOMPARE(fileLine->geometry().right(), file->width() - 1);
        // A track parameter picks a picture track or none; a sound one a
        // sound track or none.
        auto *track = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_map"));
        auto *sound = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_audioLayer"));
        QVERIFY(track && sound);
        QCOMPARE(inspector->m_pictureTracks, (QStringList{"V1", "V2"}));
        QCOMPARE(track->count(), 3);
        QCOMPARE(track->itemText(2), QStringLiteral("V2"));
        QCOMPARE(inspector->m_soundTracks, QStringList{"A1"});
        QCOMPARE(sound->count(), 2);
        track->setCurrentIndex(2);
        emit track->activated(2);
        const QJsonObject picked = takeOps(window).first().toObject();
        QCOMPARE(picked.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(picked.value("params").toObject().value("map").toString(), QStringLiteral("V2"));
    }

    static void keyWith(QWidget *widget, int code, Qt::KeyboardModifiers modifiers) {
        QKeyEvent event(QEvent::KeyPress, code, modifiers);
        QCoreApplication::sendEvent(widget, &event);
    }

    // Editing in the Inspector never strands the transport: a press or a
    // drag on a field takes no keyboard focus, and Return or Esc after
    // typing (or Esc with nothing typed) hands the keys back to the
    // timeline, so Space plays at once.
    void inspectorEditsLeaveTheTransportKeys() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        window.m_errorBanner->hide();
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QTRY_VERIFY(window.m_timeline->hasFocus());
        ScrubField *x = window.m_inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        ScrubField *opacity = window.m_inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QVERIFY(x && opacity && x->isVisible() && opacity->isVisible());
        QCOMPARE(x->focusPolicy(), Qt::TabFocus);
        const auto space = [&window] {
            window.m_lastCommand = {};
            QTest::keyClick(QApplication::focusWidget(), Qt::Key_Space);
            return window.m_lastCommand.value(QStringLiteral("type")).toString();
        };
        // Drag-scrub Position X by 20 points: the timeline keeps the keys.
        QTest::mousePress(x, Qt::LeftButton, {}, QPoint(30, 12));
        QTest::mouseMove(x, QPoint(40, 12));
        QTest::mouseMove(x, QPoint(50, 12));
        QTest::mouseRelease(x, Qt::LeftButton, {}, QPoint(50, 12));
        QVERIFY(window.m_timeline->hasFocus());
        QCOMPARE(space(), QStringLiteral("play"));
        // Type 50 and Return in Opacity: the edit goes, then the keys go back.
        QTest::mouseClick(opacity, Qt::LeftButton, {}, QPoint(30, 12));
        QVERIFY(opacity->isTyping() && opacity->editor()->hasFocus());
        opacity->editor()->setText(QStringLiteral("50"));
        QTest::keyClick(opacity->editor(), Qt::Key_Return);
        QVERIFY(!opacity->isTyping());
        QVERIFY(window.m_timeline->hasFocus());
        QCOMPARE(space(), QStringLiteral("play"));
        // Esc while typing reverts and hands the keys back too.
        QTest::mouseClick(opacity, Qt::LeftButton, {}, QPoint(30, 12));
        QVERIFY(opacity->isTyping());
        QTest::keyClick(opacity->editor(), Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
        // Tab reaches a field (and types in it); Esc there, then Esc again
        // anywhere in the Inspector, gives the keys back.
        x->setFocus(Qt::TabFocusReason);
        QVERIFY(x->isTyping());
        QTest::keyClick(x->editor(), Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
        window.m_inspector->setFocus(Qt::OtherFocusReason);
        QTest::keyClick(window.m_inspector, Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
    }

    // A ScrubField scrubs by dragging its value or its row's label (one
    // step per 2 points; one edit, on release), steps with ↑↓ (⇧ ×10,
    // ⌥ ÷10), takes typed values with units, relative and clamped, and draws
    // a value at its default quieter than a changed one.
    void inspectorScrubFieldsDragStepAndType() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props) {
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", props}}}}}}}}}});
        };
        snapshot(1, {});
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        ScrubField *x = inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QVERIFY(x && opacity);
        x->resize(78, ScrubField::kHeight);
        // A drag of 20 points is 10 steps of 1 px: one set, on release.
        QSignalSpy previews(x, &ScrubField::previewed);
        mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
        mouse(x, QEvent::MouseMove, QPointF(40, 12));
        mouse(x, QEvent::MouseMove, QPointF(50, 12));
        QVERIFY(previews.size() >= 2);
        QVERIFY(takeOps(window).isEmpty());
        QCOMPARE(x->text(), QStringLiteral("10"));
        mouse(x, QEvent::MouseButtonRelease, QPointF(50, 12));
        QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.first().toObject().value("op").toString(), QStringLiteral("set"));
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 10.0 / 1080.0) < 1e-9);
        // So does its row's label.
        snapshot(2, {});
        auto *row = qobject_cast<InspectorRow *>(x->parentWidget());
        QVERIFY(row && row->label()->field() == x);
        mouse(row->label(), QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(row->label(), QEvent::MouseMove, QPointF(30, 12));
        mouse(row->label(), QEvent::MouseButtonRelease, QPointF(30, 12));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 10.0 / 1080.0) < 1e-9);
        // A click without a drag types: the number, selected.
        snapshot(3, {});
        mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
        mouse(x, QEvent::MouseButtonRelease, QPointF(31, 12));
        QVERIFY(x->isTyping());
        QCOMPARE(x->editor()->selectedText(), QStringLiteral("0"));
        key(x->editor(), Qt::Key_Escape);
        QVERIFY(!x->isTyping());
        QVERIFY(takeOps(window).isEmpty());
        // ↑ steps 1 px, ⇧↑ 10, ⌥↑ 0.1, where the keys are after a click on
        // the field (its line edit, which would only move its caret): the
        // text follows each step, selected, and the run with its Return is
        // one edit.
        const auto clickOpen = [&] {
            mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
            mouse(x, QEvent::MouseButtonRelease, QPointF(30, 12));
            QVERIFY(x->isTyping());
        };
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::NoModifier);
        QVERIFY(x->isStepping());
        QCOMPARE(x->editor()->text(), QStringLiteral("1"));
        QCOMPARE(x->editor()->selectedText(), QStringLiteral("1"));
        QVERIFY(takeOps(window).isEmpty());
        key(x->editor(), Qt::Key_Return);
        QVERIFY(!x->isTyping() && !x->isStepping());
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 1.0 / 1080.0) < 1e-9);
        QVERIFY(takeOps(window).isEmpty());
        snapshot(4, QJsonObject{{"x", 1.0 / 1080.0}});
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::ShiftModifier);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 11.0 / 1080.0) < 1e-9);
        snapshot(5, QJsonObject{{"x", 11.0 / 1080.0}});
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::AltModifier);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 11.1 / 1080.0) < 1e-9);
        // ↓ from a typed value; Esc takes back a run not yet sent.
        snapshot(6, QJsonObject{{"x", 0.0}});
        clickOpen();
        x->editor()->setText(QStringLiteral("20"));
        keyWith(x->editor(), Qt::Key_Down, Qt::NoModifier);
        QCOMPARE(x->editor()->text(), QStringLiteral("19"));
        key(x->editor(), Qt::Key_Escape);
        QVERIFY(!x->isStepping());
        QCOMPARE(x->text(), QStringLiteral("0"));
        QVERIFY(takeOps(window).isEmpty());
        // Held: thirty repeats are one edit (one undo step), sent once the
        // run pauses, and a snapshot arriving mid-run does not pull the value
        // back; Return then sends nothing more.
        clickOpen();
        for (int repeat = 0; repeat < 30; ++repeat) {
            QKeyEvent press(QEvent::KeyPress, Qt::Key_Up, Qt::NoModifier, QString(), repeat > 0);
            QCoreApplication::sendEvent(x->editor(), &press);
            if (repeat == 10) x->setValue(0.0);
        }
        QCOMPARE(x->editor()->text(), QStringLiteral("30"));
        QVERIFY(takeOps(window).isEmpty());
        QTRY_VERIFY_WITH_TIMEOUT(!x->isStepping(), ScrubField::kStepRunMs * 4);
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 30.0 / 1080.0) < 1e-9);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(takeOps(window).isEmpty());
        // Focused without typing (Return closed the editor and the keys have
        // not gone back yet), ↑ steps the field itself as well.
        snapshot(7, QJsonObject{{"x", 30.0 / 1080.0}});
        keyWith(x, Qt::Key_Up, Qt::NoModifier);
        x->flushSteps();
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 31.0 / 1080.0) < 1e-9);
        // Typed: 50% is 0.5; +5 is relative; out of range clamps, and the
        // field flashes.
        opacity->commitText(QStringLiteral("50%"));
        QCOMPARE(takeOps(window).first().toObject().value("props").toObject().value("opacity").toDouble(), 0.5);
        snapshot(16, QJsonObject{{"opacity", 0.5}});
        opacity->commitText(QStringLiteral("+5"));
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("opacity").toDouble() - 0.55) < 1e-9);
        snapshot(17, QJsonObject{{"opacity", 0.5}});
        opacity->commitText(QStringLiteral("250"));
        QVERIFY(takeOps(window).first().toObject().value("props").toObject().value("opacity").isNull());
        QVERIFY(opacity->m_flash->isActive());
        QCOMPARE(opacity->text(), QStringLiteral("100"));
        // Parsing and formatting, units included.
        NumberFormat percent = opacity->format();
        QCOMPARE(parseNumber(percent, QStringLiteral("1.5x"), 100.0).value(), 150.0);
        QCOMPARE(parseNumber(percent, QStringLiteral("*2"), 40.0).value(), 80.0);
        QCOMPARE(formatWithUnit(percent, 50.0), QStringLiteral("50%"));
        QCOMPARE(formatWithUnit(x->format(), -48.0), QString(QChar(0x2212)) + QStringLiteral("48 px"));
        QVERIFY(!parseNumber(percent, QStringLiteral("soon"), 1.0));
        // A value at its default is drawn in the secondary ink, a changed one
        // in the body's.
        snapshot(18, QJsonObject{{"x", 0.1}});
        ScrubField *y = inspector->findChild<ScrubField *>(QStringLiteral("prop_y"));
        y->resize(78, ScrubField::kHeight);
        x->resize(78, ScrubField::kHeight);
        TextRecorder quiet;
        y->render(&quiet);
        QCOMPARE(quiet.colorOf(QStringLiteral("0")).rgb(), editorTheme().textSecondary.rgb());
        TextRecorder changed;
        x->render(&changed);
        QCOMPARE(changed.colorOf(QStringLiteral("108")).rgb(), editorTheme().text.rgb());
        QCOMPARE(changed.colorOf(QStringLiteral("px")).rgb(), editorTheme().textSecondary.rgb());
    }

    // ↺ shows on a row's hover only while its value differs from the
    // default, and sends null; so does double-clicking its label. A
    // section's ↺ resets all its rows in one edit, and its folded header
    // says what changed in it.
    void inspectorRowsResetToDefaults() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props) {
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", props}}}}}}}}}});
        };
        snapshot(1, QJsonObject{{"scale", 1.12}, {"crop_l", 0.1}, {"crop_b", 0.05}});
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        auto *scaleRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"))->parentWidget());
        auto *opacityRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"))->parentWidget());
        QVERIFY(scaleRow && opacityRow);
        const auto hover = [](QWidget *widget, bool in) {
            if (in) {
                QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), QPointF(4, 4));
                QCoreApplication::sendEvent(widget, &enter);
            } else {
                QEvent leave(QEvent::Leave);
                QCoreApplication::sendEvent(widget, &leave);
            }
        };
        QVERIFY(scaleRow->isChanged() && !opacityRow->isChanged());
        QVERIFY(!scaleRow->resetButton()->isVisibleTo(scaleRow));
        hover(scaleRow, true);
        QVERIFY(scaleRow->resetButton()->isVisibleTo(scaleRow));
        hover(opacityRow, true);
        QVERIFY(!opacityRow->resetButton()->isVisibleTo(opacityRow));
        scaleRow->resetButton()->click();
        QJsonObject op = takeOps(window).first().toObject();
        QVERIFY(op.value("props").toObject().contains("scale") && op.value("props").toObject().value("scale").isNull());
        hover(scaleRow, false);
        QVERIFY(!scaleRow->resetButton()->isVisibleTo(scaleRow));
        // Double-clicking the label resets it too.
        QMouseEvent doubleClick(QEvent::MouseButtonDblClick, QPointF(4, 4), scaleRow->label()->mapToGlobal(QPointF(4, 4)),
                                Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(scaleRow->label(), &doubleClick);
        QVERIFY(takeOps(window).first().toObject().value("props").toObject().value("scale").isNull());
        // Crop folds to what changed in it, and resets in one edit.
        InspectorSection *crop = inspector->section(QStringLiteral("crop"));
        QVERIFY(crop && !crop->isOpen());
        QCOMPARE(crop->summary(), QStringLiteral("L 192 · B 54 px"));
        hover(crop->header(), true);
        QVERIFY(crop->resetButton()->isVisibleTo(crop));
        crop->resetButton()->click();
        const QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 2);
        for (const QJsonValue &value : ops) {
            const QJsonObject props = value.toObject().value("props").toObject();
            QCOMPARE(props.size(), 1);
            QVERIFY(props.begin().value().isNull());
        }
        snapshot(2, QJsonObject{{"crop_b", 0.05}});
        QCOMPARE(crop->summary(), QStringLiteral("Bottom 54 px"));
        snapshot(3, {});
        QVERIFY(crop->summary().isEmpty());
        hover(crop->header(), true);
        QVERIFY(!crop->resetButton()->isVisibleTo(crop));
        // A section opens and closes, and stays so for this kind of clip.
        crop->setOpen(true);
        emit crop->toggled(true, Qt::NoModifier);
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/sections/video")).toStringList().contains(QStringLiteral("crop")));
        // ⌥-click opens or closes every section; ⌘-click opens only this one.
        InspectorSection *anchor = inspector->section(QStringLiteral("anchor"));
        crop->setOpen(false);
        emit crop->toggled(false, Qt::AltModifier);
        QVERIFY(!anchor->isOpen() && !inspector->section(QStringLiteral("source"))->isOpen());
        anchor->setOpen(true);
        emit anchor->toggled(true, Qt::AltModifier);
        QVERIFY(crop->isOpen() && inspector->section(QStringLiteral("source"))->isOpen());
        emit anchor->toggled(true, Qt::ControlModifier);
        QVERIFY(anchor->isOpen() && !crop->isOpen() && !inspector->section(QStringLiteral("source"))->isOpen());
    }

    // Fit and Fill write the scale that fits or fills the frame, centred;
    // speed is a percentage, and Reverse its sign; a title's Place sets y.
    void inspectorPresetsWriteTheirValues() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props, double speed, bool reverse) {
            QJsonObject clip{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                             {"in", 0.0}, {"out", 2.0}, {"speed", speed}, {"props", props}};
            if (reverse) clip.insert("reverse", true);
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()}, {"generators", catalogGenerators()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{
                    QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                    QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "t"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                    {"props", QJsonObject{{"text", "Lower"}}}}}}}}}}}});
        };
        snapshot(1, {}, 1.0, false);
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        auto *fit = inspector->findChild<SegmentedField *>(QStringLiteral("fitFill"));
        QVERIFY(fit);
        QCOMPARE(fit->current(), 0);
        emit fit->activated(1);
        QJsonObject props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY2(std::abs(props.value("scale").toDouble() - 3.1605) < 0.001, qPrintable(QString::number(props.value("scale").toDouble())));
        QVERIFY(props.contains("x") && props.value("x").isNull() && props.contains("y") && props.value("y").isNull());
        snapshot(2, QJsonObject{{"scale", 3.1605}}, 1.0, false);
        QCOMPARE(fit->current(), 1);
        emit fit->activated(0);
        props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY(props.value("scale").isNull() && props.value("x").isNull());
        snapshot(3, QJsonObject{{"scale", 3.1605}, {"x", 0.1}}, 1.0, false);
        QCOMPARE(fit->current(), -1);
        // Speed: 200% is a ratio of 2; Reverse its sign; presets keep the direction.
        ScrubField *speed = inspector->findChild<ScrubField *>(QStringLiteral("speed"));
        auto *reverse = inspector->findChild<GlyphButton *>(QStringLiteral("speedReverse"));
        QVERIFY(speed && reverse);
        QCOMPARE(speed->text(), QStringLiteral("100"));
        // Off, Reverse is a quiet outline in the secondary ink, no disc.
        QVERIFY(!reverse->isChecked());
        QCOMPARE(reverse->inkColor(), editorTheme().textSecondary);
        QVERIFY(!reverse->discColor().isValid());
        speed->commitText(QStringLiteral("200"));
        QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("speed"));
        QCOMPARE(op.value("speed").toString(), QStringLiteral("2"));
        snapshot(4, {}, 2.0, false);
        reverse->click();
        QCOMPARE(takeOps(window).first().toObject().value("speed").toString(), QStringLiteral("-2"));
        snapshot(5, {}, 2.0, true);
        QCOMPARE(speed->text(), QString(QChar(0x2212)) + QStringLiteral("200"));
        QVERIFY(reverse->isChecked());
        // On: full ink on the neutral pressed disc, no purple wash.
        QCOMPARE(reverse->inkColor(), editorTheme().text);
        QCOMPARE(reverse->discColor(), editorTheme().pressed);
        QVERIFY(reverse->discColor() != editorTheme().accentTint);
        inspector->findChild<QAction *>(QStringLiteral("speed50"))->trigger();
        QCOMPARE(takeOps(window).first().toObject().value("speed").toString(), QStringLiteral("-0.5"));
        // A title's Place: Top is y −0.30.
        window.selectClip(QStringLiteral("t"));
        auto *place = inspector->findChild<SegmentedField *>(QStringLiteral("titlePlace"));
        QVERIFY(place);
        QCOMPARE(place->current(), 1);
        emit place->activated(0);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("y").toDouble() + 0.30) < 1e-9);
        auto *align = inspector->findChild<SegmentedField *>(QStringLiteral("titleAlign"));
        QVERIFY(align);
        QCOMPARE(align->current(), 1);
        emit align->activated(0);
        QCOMPARE(takeOps(window).first().toObject().value("props").toObject().value("align").toString(), QStringLiteral("left"));
    }

    // Start, End and Duration are timecode fields on the sequence's rate:
    // Start moves the clip, End and Duration trim its out point; Source In
    // slips it on the media's rate. Typed timecode may be short, relative,
    // or seconds. Drop-frame rates show drop-frame timecode.
    void inspectorTimingSendsTimecodeEdits() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 20}, {"timeline", QJsonObject{
            {"fps", 24}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 6.0}, {"duration", 5.0},
                            {"startFrame", 144}, {"durationFrames", 120}, {"in", 2.0}, {"out", 7.0}, {"speed", 1.0}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        ScrubField *start = inspector->findChild<ScrubField *>(QStringLiteral("timing_start"));
        ScrubField *end = inspector->findChild<ScrubField *>(QStringLiteral("timing_end"));
        ScrubField *duration = inspector->findChild<ScrubField *>(QStringLiteral("timing_duration"));
        ScrubField *sourceIn = inspector->findChild<ScrubField *>(QStringLiteral("source_in"));
        QVERIFY(start && end && duration && sourceIn);
        QCOMPARE(start->text(), QStringLiteral("00:00:06:00"));
        QCOMPARE(end->text(), QStringLiteral("00:00:11:00"));
        QCOMPARE(duration->text(), QStringLiteral("00:00:05:00"));
        QCOMPARE(sourceIn->text(), QStringLiteral("00:00:02:00"));
        // Five spin boxes became three timecode fields: no spin boxes at all.
        QVERIFY(inspector->findChildren<QDoubleSpinBox *>().isEmpty());
        const auto edit = [&window] { return takeOps(window).first().toObject(); };
        start->commitText(QStringLiteral("00:00:07:00"));
        QJsonObject op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("move"));
        QCOMPARE(op.value("frames").toInt(), 24);
        start->setValue(144);
        start->commitText(QStringLiteral("+12"));
        QCOMPARE(edit().value("frames").toInt(), 12);
        end->commitText(QStringLiteral("10:00"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op.value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(op.value("frames").toInt(), -24);
        duration->commitText(QStringLiteral("4s"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op.value("frames").toInt(), -24);
        sourceIn->commitText(QStringLiteral("00:00:03:00"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("slip"));
        QCOMPARE(op.value("frames").toInt(), 24);
        // 29.97 drop-frame.
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 120}, {"timeline", QJsonObject{
            {"fps", 30000.0 / 1001.0}, {"dropFrame", true},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 60.06}, {"duration", 5.0},
                            {"startFrame", 1800}, {"durationFrames", 150}}}}}}}}}});
        start = inspector->findChild<ScrubField *>(QStringLiteral("timing_start"));
        QCOMPARE(start->text(), QStringLiteral("00:01:00;02"));
        QCOMPARE(frameTimecode(17982, 30000.0 / 1001.0, true), QStringLiteral("00:10:00;00"));
        QCOMPARE(parseTimecode(QStringLiteral("00:01:00;02"), 30000.0 / 1001.0, true, nullptr).value(), qint64(1800));
        QCOMPARE(parseTimecode(QStringLiteral("7:00"), 24, false, nullptr).value(), qint64(168));
        bool relative = false;
        QCOMPARE(parseTimecode(QStringLiteral("-1:00"), 24, false, &relative).value(), qint64(-24));
        QVERIFY(relative);
    }

    // A project with a clip of every kind the Inspector names.
    static QJsonObject everyKindSnapshot(int revision) {
        const QJsonArray sources{
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24},
                        {"hasAudio", true}, {"channels", 2}, {"sampleRate", 48000}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}, {"hasAudio", true}, {"channels", 2}, {"sampleRate", 48000}}};
        const auto clip = [](const char *id, double start, double duration, QJsonObject extra) {
            extra.insert("id", id);
            extra.insert("timelineStart", start);
            extra.insert("duration", duration);
            extra.insert("startFrame", std::llround(start * 24));
            extra.insert("durationFrames", std::llround(duration * 24));
            return extra;
        };
        return QJsonObject{{"revision", revision}, {"duration", 18}, {"timeline", QJsonObject{
            {"name", "Harbor Teaser"}, {"fps", 24}, {"width", 1080}, {"height", 1920}, {"sampleRate", 48000},
            {"properties", catalogProperties()}, {"generators", catalogGenerators()}, {"sources", sources},
            {"maskModes", QJsonArray{"add", "subtract", "none"}},
            {"markers", QJsonArray{QJsonObject{{"id", "m1"}, {"time", 6.0}, {"duration", 0.0}, {"color", "red"}, {"label", "Boat"}},
                                   QJsonObject{{"id", "m2"}, {"time", 13.0}, {"duration", 2.0}, {"color", "green"}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"name", "Video 1"}, {"clips", QJsonArray{
                    clip("v", 6.0, 5.0, {{"kind", "media"}, {"sourceId", "close"}, {"name", "fisher-close.mov"}, {"in", 2.0}, {"out", 7.0},
                                         {"speed", 1.0}, {"link", "L1"}}),
                    clip("n", 11.0, 4.0, {{"kind", "sequence"}, {"sequenceId", "s2"}, {"name", "Harbor B-roll"}})}}},
                QJsonObject{{"id", "V2"}, {"type", "video"}, {"name", "Titles"}, {"clips", QJsonArray{
                    clip("t", 1.0, 4.0, {{"kind", "generator"}, {"fx", "title"}, {"name", "Harbor Teaser"}, {"props", QJsonObject{{"text", "Harbor Teaser"}}}}),
                    clip("s", 5.0, 2.0, {{"kind", "generator"}, {"fx", "solid"}, {"name", "solid"}, {"props", QJsonObject{{"color", "#335577"}}}}),
                    clip("adj", 8.0, 3.0, {{"kind", "generator"}, {"fx", "adjustment"}, {"name", "Warm grade"}})}}},
                QJsonObject{{"id", "S1"}, {"type", "subtitle"}, {"name", "Captions"}, {"clips", QJsonArray{
                    clip("cap", 2.0, 4.0, {{"kind", "caption"}, {"text", "We open on the harbor at dawn."}, {"name", "We open on the harbor at dawn."}})}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"name", "Dialogue"}, {"clips", QJsonArray{
                    clip("a", 6.0, 5.0, {{"kind", "media"}, {"sourceId", "close"}, {"name", "fisher-close.mov"}, {"in", 2.0}, {"out", 7.0},
                                         {"speed", 1.0}, {"link", "L1"}})}}},
                QJsonObject{{"id", "A2"}, {"type", "audio"}, {"name", "Music"}, {"clips", QJsonArray{
                    clip("music", 0.0, 18.0, {{"kind", "media"}, {"sourceId", "score"}, {"name", "harbor-score.wav"}, {"in", 0.0}, {"out", 18.0},
                                             {"speed", 1.0}, {"props", QJsonObject{{"gain", -8.0}}}})}}}}}}}};
    }
    static QString plainText(const QString &html) {
        static const QRegularExpression tags(QStringLiteral("<[^>]*>"));
        QString text = html;
        return text.remove(tags).replace(QStringLiteral("&amp;"), QStringLiteral("&"));
    }

    // The header never shows a stand-in name: a clip's own name, else its
    // media's file name, else what generates it, else where it is.
    void inspectorNamesEveryClip() {
        InspectorPanel inspector;
        const auto timeline = [](const QJsonObject &clip, const QJsonObject &source) {
            return QJsonObject{{"fps", 24}, {"width", 1920}, {"height", 1080}, {"sources", QJsonArray{source}},
                               {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                                                     QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}};
        };
        QJsonObject clip{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}};
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"width", 1920}, {"height", 1080}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Clip on V1"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"path", "media/harbor-wide.mov"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("harbor-wide.mov"));
        clip.insert(QStringLiteral("name"), QStringLiteral("Opening wide"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"path", "media/harbor-wide.mov"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Opening wide"));
        // A stand-in from an older editor gives way too.
        clip.insert(QStringLiteral("name"), QStringLiteral("Untitled media"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Clip on V1"));
        const QJsonObject title{{"id", "t1"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0}};
        inspector.setState(timeline(title, QJsonObject{}), {QStringLiteral("t1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Title"));
    }

    // Selecting a clip shows what it is: a thumbnail, its name, its kind
    // in the clip's own timeline colour, its track, and its Start, End and
    // Duration in timecode.
    void inspectorShowsWhatEachClipIs() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        InspectorPanel *inspector = window.m_inspector;
        const EditorTheme &theme = editorTheme();
        const struct {
            const char *id;
            const char *kind;
            const char *track;
            ClipKind color;
            const char *start;
            const char *duration;
        } cases[] = {
            {"v", "Video", "V1 Video 1 · linked A1", ClipKind::Video, "00:00:06:00", "00:00:05:00"},
            {"n", "Sequence", "V1 Video 1", ClipKind::Video, "00:00:11:00", "00:00:04:00"},
            {"t", "Title", "V2 Titles", ClipKind::Generator, "00:00:01:00", "00:00:04:00"},
            {"s", "Solid", "V2 Titles", ClipKind::Generator, "00:00:05:00", "00:00:02:00"},
            {"adj", "Adjustment", "V2 Titles", ClipKind::Generator, "00:00:08:00", "00:00:03:00"},
            // As the timeline's header names it: the subtitle track is the
            // third picture track, V3.
            {"cap", "Caption", "V3 Captions", ClipKind::Caption, "00:00:02:00", "00:00:04:00"},
            {"a", "Audio", "A1 Dialogue · linked V1", ClipKind::Audio, "00:00:06:00", "00:00:05:00"},
            {"music", "Audio", "A2 Music", ClipKind::Music, "00:00:00:00", "00:00:18:00"},
        };
        for (const auto &expected : cases) {
            const QString id = QString::fromLatin1(expected.id);
            window.selectClip(id);
            const QString kind = plainText(inspector->kindText());
            QCOMPARE(kind, QStringLiteral("%1 · %2").arg(QString::fromUtf8(expected.kind), QString::fromUtf8(expected.track)));
            QCOMPARE(inspector->m_kindSquare->color().rgb(), theme.clip(expected.color).header.rgb());
            QVERIFY(inspector->m_kindSquare->isVisibleTo(inspector));
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_start"))->text(), QString::fromLatin1(expected.start));
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_duration"))->text(), QString::fromLatin1(expected.duration));
            const QList<QImage> thumbs = inspector->m_thumbnail->images();
            QCOMPARE(thumbs.size(), 1);
            QVERIFY2(!thumbs.first().isNull(), expected.id);
            QVERIFY(!inspector->nameText().isEmpty());
        }
        // The video's thumbnail is its picture; the solid's its colour.
        window.selectClip(QStringLiteral("s"));
        const QImage solid = inspector->m_thumbnail->images().first();
        QCOMPARE(solid.pixelColor(solid.width() / 2, solid.height() / 2).rgb(), QColor(0x33, 0x55, 0x77).rgb());
        // The adjustment says what it is for; the caption how to change it.
        window.selectClip(QStringLiteral("adj"));
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Applies its effects to every layer beneath it."));
        QVERIFY(!inspector->m_note->isHidden());
        // Its effects come first, and open.
        QVERIFY(inspector->section(QStringLiteral("effects"))->isOpen());
        window.selectClip(QStringLiteral("cap"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("captionText"))->text(), QStringLiteral("We open on the harbor at dawn."));
        // Ask Kerf to change it: a link (pointing hand, no bezel), not grey text.
        auto *captionAsk = inspector->findChild<QPushButton *>(QStringLiteral("captionHint"));
        QVERIFY(captionAsk && captionAsk->property("inspectorLink").toBool());
        QCOMPARE(captionAsk->cursor().shape(), Qt::PointingHandCursor);
        QVERIFY(inspector->m_sections.isEmpty());
        // The video's Source section: its file and format, as plain text.
        window.selectClip(QStringLiteral("v"));
        QCOMPARE(inspector->section(QStringLiteral("source"))->summary(), QStringLiteral("1920 × 1080 · 24 fps"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sourceFile"))->text(), QStringLiteral("fisher-close.mov"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sourceAudio"))->text(), QStringLiteral("2 ch · 48 kHz"));
        // Its linked sound's volume and pan, edited on the linked clip.
        InspectorSection *linked = inspector->section(QStringLiteral("linkedAudio"));
        QVERIFY(linked);
        QCOMPARE(linked->title(), QStringLiteral("Audio · A1"));
        inspector->findChild<ScrubField *>(QStringLiteral("linked_gain"))->commitText(QStringLiteral("-8"));
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("clip").toString(), QStringLiteral("a"));
        QCOMPARE(op.value("props").toObject().value("gain").toDouble(), -8.0);
        // The music's folded volume reads in dB.
        window.selectClip(QStringLiteral("music"));
        QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("prop_gain"))->text(), QString(QChar(0x2212)) + QStringLiteral("8.0"));
        // Its thumbnail is its real waveform, as the bin's tile: the glyph
        // ink in the band the wave fills, the tile's colour above it.
        {
            const QImage wave = inspector->m_thumbnail->images().first();
            QCOMPARE(wave.pixelColor(64, 4).rgb(), theme.audioTile.rgb());
            int inked = 0;
            for (int x = 0; x < wave.width(); ++x) inked += wave.pixelColor(x, 36).rgb() == theme.audioTileGlyph.rgb() ? 1 : 0;
            QVERIFY2(inked > wave.width() / 2, qPrintable(QString::number(inked)));
        }
        // A sound clip carrying no effects has no Effects section: no effect
        // in this version of Kerf applies to sound.
        QVERIFY(!inspector->section(QStringLiteral("effects")));
    }

    // The name edits in place: Return renames the clip (set {name}), Esc
    // keeps it. ⋯ ▸ Ask Kerf opens the conversation about the clip, and the
    // composer names the selected clip.
    void inspectorRenamesAndAsksKerf() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        NameLabel *name = inspector->m_name;
        QCOMPARE(name->name(), QStringLiteral("fisher-close.mov"));
        QVERIFY(name->isEditable());
        QMouseEvent doubleClick(QEvent::MouseButtonDblClick, QPointF(4, 4), name->mapToGlobal(QPointF(4, 4)), Qt::LeftButton,
                                Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(name, &doubleClick);
        QVERIFY(name->editor() && name->editor()->isVisibleTo(name));
        QCOMPARE(name->editor()->text(), QStringLiteral("fisher-close.mov"));
        name->editor()->setText(QStringLiteral("Fisher close-up"));
        key(name->editor(), Qt::Key_Return);
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("v"));
        QCOMPARE(op.value("name").toString(), QStringLiteral("Fisher close-up"));
        QVERIFY(!op.contains("props"));
        name->beginEditing();
        name->editor()->setText(QStringLiteral("Nope"));
        key(name->editor(), Qt::Key_Escape);
        QVERIFY(takeOps(window).isEmpty());
        QVERIFY(!name->editor()->isVisibleTo(name));
        QCOMPARE(name->name(), QStringLiteral("fisher-close.mov"));
        // The composer names the selected clip.
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf about fisher-close.mov…"));
        // ⋯ ▸ Ask Kerf about This Clip: the conversation, open, about it.
        window.setChatOpen(false);
        QAction *ask = inspector->m_moreMenu->findChild<QAction *>(QStringLiteral("inspectorAsk"));
        QVERIFY(ask);
        ask->trigger();
        QVERIFY(window.chatOpen());
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("@fisher-close.mov "));
        window.selectClip(QString());
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Describe your next edit…"));
        // A sequence is not renamed here.
        QVERIFY(!name->isEditable());
    }

    // With nothing selected the Inspector shows the sequence: its format,
    // its tracks and its markers, as text, with only the frame size to
    // change; and a quiet hint.
    void inspectorShowsTheSequenceWithNothingSelected() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject snapshot = everyKindSnapshot(1);
        window.applySnapshot(snapshot);
        window.selectClip(QString());
        InspectorPanel *inspector = window.m_inspector;
        QCOMPARE(inspector->nameText(), QStringLiteral("Harbor Teaser"));
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("Sequence · 1080 × 1920 · 24 fps"));
        QVERIFY(!inspector->m_kindSquare->isVisibleTo(inspector));
        QVERIFY(inspector->findChildren<ScrubField *>().isEmpty());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("timing_start"))->text(), QStringLiteral("00:00:00:00"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("timing_duration"))->text(), QStringLiteral("00:00:18:00"));
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Select a clip to adjust it."));
        // The hint is its own line: 8 points under the timing row.
        {
            QLayout *header = inspector->m_header->layout();
            const int tall = header->heightForWidth(300) > 0 ? header->heightForWidth(300) : header->sizeHint().height();
            header->setGeometry(QRect(0, 0, 300, tall));
            const int below = inspector->m_note->geometry().top() + inspector->m_note->contentsMargins().top() - inspector->m_timing->geometry().bottom() - 1;
            QVERIFY2(below >= 8, qPrintable(QString::number(below)));
            const int above = tall - inspector->m_note->geometry().bottom() - 1 + inspector->m_note->contentsMargins().bottom();
            QVERIFY2(above >= 8, qPrintable(QString::number(above)));
        }
        InspectorSection *format = inspector->section(QStringLiteral("format"));
        InspectorSection *tracks = inspector->section(QStringLiteral("tracks"));
        InspectorSection *markers = inspector->section(QStringLiteral("markers"));
        QVERIFY(format && tracks && markers);
        QVERIFY(format->isOpen() && tracks->isOpen() && !markers->isOpen());
        const int trackCount = int(snapshot.value("timeline").toObject().value("tracks").toArray().size());
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("sequenceTrack")).size(), trackCount);
        // Each track's square in its timeline header's colour: a picture
        // track of titles is a titles track, as the timeline draws it.
        {
            const QJsonArray trackList = snapshot.value("timeline").toObject().value("tracks").toArray();
            const auto lines = inspector->findChildren<QWidget *>(QStringLiteral("sequenceTrack"));
            bool titles = false;
            for (int index = 0; index < trackList.size(); ++index) {
                const QJsonObject track = trackList.at(index).toObject();
                const ClipKind kind = trackKind(track);
                QCOMPARE(lines.at(index)->findChild<KindSquare *>()->color(), editorTheme().clip(kind).header);
                titles = titles || (kind == ClipKind::Generator && !track.value("type").toString().contains(QStringLiteral("title"), Qt::CaseInsensitive));
            }
            QVERIFY2(titles, "a picture track holding titles");
        }
        QCOMPARE(tracks->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QString::number(trackCount));
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("sequenceMarker")).size(), 2);
        QCOMPARE(markers->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("2"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sequenceRate"))->text(), QStringLiteral("24 fps"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sequenceAudio"))->text(), QStringLiteral("48 kHz"));
        // Only the frame size can change; no clip fields.
        const auto popups = inspector->findChildren<QComboBox *>();
        QCOMPARE(popups.size(), 1);
        QComboBox *size = popups.first();
        QCOMPARE(size->objectName(), QStringLiteral("frameSize"));
        // Written as the header writes it (thin spaces round the ×).
        QCOMPARE(size->currentText(), QStringLiteral("9:16 · 1080\u2009×\u20091920"));
        QVERIFY(inspector->findChildren<QLineEdit *>().size() <= 1); // the name's editor, hidden
        QVERIFY(inspector->findChildren<QPlainTextEdit *>().isEmpty());
        QVERIFY(inspector->findChildren<InspectorSwitch *>().isEmpty());
        size->setCurrentIndex(0);
        emit size->activated(0);
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set-sequence"));
        QCOMPARE(op.value("width").toInt(), 1920);
        QCOMPARE(op.value("height").toInt(), 1080);
    }

    // Several clips: "3 clips", what they share, "Mixed" where they differ;
    // a drag moves each by the same amount and typing sets each to the same
    // value, either way in one edit. Clips of different kinds share only
    // their timing.
    void inspectorShowsWhatSeveralClipsShare() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto clip = [](const char *id, double start, const QJsonObject &props) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", start}, {"duration", 2.0},
                               {"startFrame", std::llround(start * 24)}, {"durationFrames", 48}, {"props", props}};
        };
        QJsonObject first = clip("v1", 0.0, QJsonObject{{"opacity", 0.5}});
        first.insert("effects", QJsonArray{QJsonObject{{"fx", "blur.gaussian"}, {"enabled", true}, {"params", QJsonObject{}}}});
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    first, clip("v2", 2.0, QJsonObject{{"opacity", 0.5}, {"x", 0.1}}), clip("v3", 4.0, QJsonObject{{"opacity", 1.0}})}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "a1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"startFrame", 0}, {"durationFrames", 48}}}}}}}}}});
        InspectorPanel *inspector = window.m_inspector;
        inspector->setSelection({QStringLiteral("v1"), QStringLiteral("v2"), QStringLiteral("v3")});
        QCOMPARE(inspector->nameText(), QStringLiteral("3 clips"));
        QVERIFY(!inspector->m_name->isEditable());
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("Video · V1"));
        QCOMPARE(inspector->m_thumbnail->images().size(), 3);
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        ScrubField *x = inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        QVERIFY(opacity && x);
        QVERIFY(opacity->isMixed() && x->isMixed());
        QCOMPARE(opacity->text(), QStringLiteral("Mixed"));
        QVERIFY(inspector->findChild<InspectorSlider *>(QStringLiteral("slider_opacity"))->m_mixed);
        QCOMPARE(inspector->section(QStringLiteral("effects"))->summary(), QStringLiteral("On 1 of 3 clips"));
        // A drag of 20 points moves each clip 10 px, in one edit.
        x->resize(78, ScrubField::kHeight);
        mouse(x, QEvent::MouseButtonPress, QPointF(20, 12));
        mouse(x, QEvent::MouseMove, QPointF(40, 12));
        mouse(x, QEvent::MouseButtonRelease, QPointF(40, 12));
        QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        const QList<double> before{0.0, 192.0, 0.0};
        for (int index = 0; index < 3; ++index) {
            const QJsonObject op = ops.at(index).toObject();
            QCOMPARE(op.value("clip").toString(), QStringLiteral("v%1").arg(index + 1));
            QVERIFY(std::abs(op.value("props").toObject().value("x").toDouble() * 1920.0 - (before.at(index) + 10.0)) < 1e-6);
        }
        // Typing sets each to the value.
        x->commitText(QStringLiteral("50"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        for (const QJsonValue &op : ops) QVERIFY(std::abs(op.toObject().value("props").toObject().value("x").toDouble() * 1920.0 - 50.0) < 1e-6);
        // The timing moves them all.
        inspector->findChild<ScrubField *>(QStringLiteral("timing_start"))->commitText(QStringLiteral("+12"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        for (const QJsonValue &op : ops) {
            QCOMPARE(op.toObject().value("op").toString(), QStringLiteral("move"));
            QCOMPARE(op.toObject().value("frames").toInt(), 12);
        }
        // End is the span's end: a new End moves it by trimming only the
        // clip that ends there (v3), so no gap opens after v1 or v2.
        ScrubField *spanEnd = inspector->findChild<ScrubField *>(QStringLiteral("timing_end"));
        QCOMPARE(spanEnd->value(), 144.0);
        spanEnd->commitText(QStringLiteral("00:00:05:00"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.at(0).toObject().value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(ops.at(0).toObject().value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(ops.at(0).toObject().value("clip").toString(), QStringLiteral("v3"));
        QCOMPARE(ops.at(0).toObject().value("frames").toInt(), -24);
        spanEnd->commitText(QStringLiteral("+12"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.at(0).toObject().value("clip").toString(), QStringLiteral("v3"));
        QCOMPARE(ops.at(0).toObject().value("frames").toInt(), 12);
        // The timing row keeps one alignment, timecode and "Mixed" alike.
        for (ScrubField *field : std::as_const(inspector->m_timingFields)) QCOMPARE(field->valueAlignment(), Qt::Alignment(Qt::AlignLeft));
        // Different kinds: the timing only, and why.
        inspector->setSelection({QStringLiteral("v1"), QStringLiteral("a1")});
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("1 Audio, 1 Video"));
        QVERIFY(inspector->findChildren<InspectorRow *>().isEmpty());
        QCOMPARE(inspector->m_timingFields.size(), 3);
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Video and audio clips have no properties in common."));
    }

    // Selecting another clip of the same shape only changes values: the
    // same row widgets, open sections and scroll offset.
    void inspectorReusesRowsForClipsOfOneShape() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonArray stack;
        for (int index = 0; index < 10; ++index)
            stack.append(QJsonObject{{"fx", index % 2 ? "color.basic" : "blur.gaussian"}, {"enabled", true}, {"params", QJsonObject{}}});
        const auto clip = [&stack](const char *id, double start, double opacity) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", start}, {"duration", 2.0},
                               {"props", QJsonObject{{"opacity", opacity}}}, {"effects", stack}};
        };
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("c1", 0, 0.5), clip("c2", 2, 0.25)}}}}}}}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->isVisible());
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QCOMPARE(opacity->text(), QStringLiteral("50"));
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        inspector->section(QStringLiteral("crop"))->setOpen(true);
        QCoreApplication::processEvents();
        QScrollBar *scroll = inspector->scrollArea()->verticalScrollBar();
        // An overlay-style scroller: 6 points, no groove, its handle shown
        // only while the pointer is over the Inspector (or it is wheeled).
        {
            QCOMPARE(scroll->sizeHint().width(), 6);
            QVERIFY(!inspector->scrollArea()->property("active").toBool());
            QEnterEvent enter(QPointF(10, 10), QPointF(10, 10), QPointF(10, 10));
            QCoreApplication::sendEvent(inspector, &enter);
            QVERIFY(inspector->scrollArea()->property("active").toBool());
            QEvent leave(QEvent::Leave);
            QCoreApplication::sendEvent(inspector, &leave);
            QVERIFY(!inspector->scrollArea()->property("active").toBool());
        }
        QVERIFY(scroll->maximum() > 60);
        scroll->setValue(60);
        // From the click to the painted values (the panel repainted at once,
        // not when the event loop gets to it).
        const auto timedSwitch = [&](const char *id) {
            QElapsedTimer timer;
            timer.start();
            window.clickSelectClip(QString::fromLatin1(id));
            inspector->repaint();
            return timer.nsecsElapsed() / 1.0e6;
        };
        const double elapsed = timedSwitch("c2");
        QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity")), opacity);
        QCOMPARE(opacity->text(), QStringLiteral("25"));
        QVERIFY(inspector->section(QStringLiteral("effects"))->isOpen() && inspector->section(QStringLiteral("crop"))->isOpen());
        QCoreApplication::processEvents();
        QCOMPARE(scroll->value(), 60);
        // Values only: well under a frame, even with ten effects. The median
        // of a run of switches back and forth, so one slow scheduling slice
        // on a shared CI machine does not decide it. The target is 8 ms; the
        // check allows twice that, as CI builds the tests unoptimized on
        // shared runners (7.9 ms measured unoptimized on an M-series Mac,
        // offscreen; see the README).
        QList<double> times{elapsed};
        for (int index = 0; index < 8; ++index) times << timedSwitch(index % 2 ? "c2" : "c1");
        std::sort(times.begin(), times.end());
        const double median = times.at(times.size() / 2);
        qInfo("Inspector: same-shape clip switch, click to painted values: median %.2f ms, fastest %.2f ms (10 effects)", median,
              times.first());
        QVERIFY2(median < 16.0, qPrintable(QStringLiteral("%1 ms").arg(median)));
        QCOMPARE(scroll->value(), 60);
        // The header's hairline shows while the body is scrolled under it.
        QVERIFY(inspector->m_scrollEdge->isVisible());
        scroll->setValue(0);
        QVERIFY(!inspector->m_scrollEdge->isVisible());
    }

    // Clicking a clip brings the Inspector to the front of its column and
    // remembers the tab it came from; clearing the selection goes back to
    // it, unless the Inspector was chosen by hand. A clip selected by an
    // edit raises nothing, and a closed column stays closed. The
    // conversation is never touched, and keyboard focus stays put.
    void inspectorFollowsClickedClips() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspectorFollowsSelection"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspectorFollowsSelection"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        ads::CDockWidget *inspector = workspace->panel(QStringLiteral("inspector"));
        ads::CDockWidget *project = workspace->panel(QStringLiteral("project"));
        QVERIFY(project->isCurrentTab());
        const QRect chat = window.m_chat->geometry();
        QVERIFY(window.m_chat->isVisible());
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QCoreApplication::processEvents();
        QWidget *focus = QApplication::focusWidget();
        // A click on a clip: the Inspector, showing it; focus stays.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        QCOMPARE(window.m_inspector->nameText(), QStringLiteral("fisher-close.mov"));
        QCOMPARE(QApplication::focusWidget(), focus);
        // Another clip: only the values change.
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        // Deselected (an empty lane, Esc): back to Project.
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        QVERIFY(window.m_selectedClip.isEmpty());
        // A selection by an edit raises nothing.
        window.selectClip(QStringLiteral("v1b"));
        QVERIFY(project->isCurrentTab());
        window.selectClip(QString());
        // Project chosen while a clip is selected stays until a new clip is clicked.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isCurrentTab());
        project->setAsCurrentTab();
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QVERIFY(project->isCurrentTab());
        // ⌘4: the Inspector by hand, which stays when the selection clears
        // and shows the sequence; ⌘4 again goes back.
        QAction *show = window.m_commands->action(QStringLiteral("window.inspector"));
        show->trigger();
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        QVERIFY(show->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        emit window.m_timeline->deselectRequested();
        QVERIFY(inspector->isCurrentTab());
        QCOMPARE(window.m_inspector->m_mode, InspectorPanel::Mode::Sequence);
        show->trigger();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        QVERIFY(!inspector->isClosed() && show->isChecked());
        // Following off: clicks leave the column alone, and it is remembered.
        QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
        QVERIFY(follows->isCheckable() && follows->isChecked());
        follows->trigger();
        QVERIFY(!QSettings().value(QStringLiteral("editor/inspectorFollowsSelection")).toBool());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(project->isCurrentTab());
        follows->trigger();
        QVERIFY(Workspace::inspectorFollowsSelection());
        emit window.m_timeline->deselectRequested();
        // A closed Inspector is not reopened by a click.
        inspector->toggleView(false);
        QVERIFY(!window.m_commands->action(QStringLiteral("window.inspector"))->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isClosed());
        // ⌘4 opens it again.
        show->trigger();
        QVERIFY(!inspector->isClosed() && inspector->isCurrentTab());
        // Effects or Presets chosen by hand stay in front when another clip
        // is clicked (looks applied clip by clip), and the browser adds to
        // the clip just clicked.
        ads::CDockWidget *effects = workspace->panel(QStringLiteral("effects"));
        ads::CDockWidget *presets = workspace->panel(QStringLiteral("presets"));
        QVERIFY(effects && presets);
        QCOMPARE(effects->dockAreaWidget(), inspector->dockAreaWidget());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isCurrentTab());
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        QCOMPARE(window.m_inspector->selection(), QStringList{QStringLiteral("v1a")});
        takeOps(window);
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        {
            const QJsonArray added = takeOps(window);
            QCOMPARE(added.size(), 1);
            QCOMPARE(added.at(0).toObject().value("op").toString(), QStringLiteral("fx-add"));
            QCOMPARE(added.at(0).toObject().value("clip").toString(), QStringLiteral("v1a"));
        }
        QVERIFY(effects->isCurrentTab());
        presets->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(presets->isCurrentTab());
        // From Project a click still raises it; the Inspector's own Add
        // effect brings Effects up for it, and the next click brings the
        // Inspector back.
        project->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        emit window.m_inspector->addEffectRequested(false);
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // Add effect, an effect added (the Inspector comes back), then a
        // deselect: back to Project, not Effects; the next click raises
        // the Inspector again.
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        emit window.m_inspector->addEffectRequested(false);
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // It still knows the clip it shows: Project chosen by hand stays
        // when that same clip is clicked again (to drag or trim it); a new
        // clip brings the Inspector back.
        project->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // Effects chosen by hand stays through a deselect and the next
        // click; Project then gives the click back to the Inspector.
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        project->setAsCurrentTab();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // ⌘4 still shows it over a browser chosen by hand.
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        show->trigger();
        QVERIFY(inspector->isCurrentTab());
        // The conversation never moved.
        QVERIFY(window.m_chat->isVisible());
        QCOMPARE(window.m_chat->geometry(), chat);
    }

    // Esc on the timeline asks to clear the selection (when no proposal
    // takes Esc for Reject); so does a click on an empty lane.
    void timelineAsksToClearTheSelection() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 1.0}, {"name", "Opening"}}}}}}}}, 8, 24);
        QSignalSpy cleared(&timeline, &TimelineWidget::deselectRequested);
        key(&timeline, Qt::Key_Escape);
        QCOMPARE(cleared.size(), 0);
        timeline.setSelectedClip(QStringLiteral("clip"));
        key(&timeline, Qt::Key_Escape);
        QCOMPARE(cleared.size(), 1);
        mouse(&timeline, QEvent::MouseButtonPress, QPointF(kTimelineOrigin + 400, kRulerHeight + kTrackHeight / 2));
        mouse(&timeline, QEvent::MouseButtonRelease, QPointF(kTimelineOrigin + 400, kRulerHeight + kTrackHeight / 2));
        QCOMPARE(cleared.size(), 2);
    }

    // ⌘4 is Window ▸ Inspector (Effect Controls' old command still finds
    // it); ⌥⌘4 is Inspector Follows Selection. Workspaces saved with Effect
    // Controls restore with the Inspector where it was.
    void inspectorCommandsAndLayoutMigration() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray old;
        {
            MainWindow window{QString(), QString()};
            QAction *inspector = window.m_commands->action(QStringLiteral("window.inspector"));
            QVERIFY(inspector && inspector->isCheckable());
            QCOMPARE(inspector->text(), QStringLiteral("Inspector"));
            QCOMPARE(inspector->shortcut(), QKeySequence(Qt::CTRL | Qt::Key_4));
            QCOMPARE(window.m_commands->action(QStringLiteral("window.effectControls")), inspector);
            QCOMPARE(CommandRegistry::resolve(QStringLiteral("window.effectControls")), QStringLiteral("window.inspector"));
            QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
            QVERIFY(follows && follows->isCheckable());
            QCOMPARE(follows->shortcut(), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_4));
            // Its tab is second in the column, named Inspector.
            ads::CDockAreaWidget *area = window.m_workspace->panel(QStringLiteral("project"))->dockAreaWidget();
            QCOMPARE(area->dockWidget(1)->objectName(), QStringLiteral("inspector"));
            QCOMPARE(area->dockWidget(1)->windowTitle(), QStringLiteral("Inspector"));
            // A version-6 layout, with the Inspector by its old id and Presets closed.
            window.m_workspace->panel(QStringLiteral("presets"))->toggleView(false);
            old = window.m_workspace->manager()->saveState(6);
            if (!old.startsWith("<?xml")) old = qUncompress(old);
            QVERIFY(old.contains("Name=\"inspector\""));
            old.replace("Name=\"inspector\"", "Name=\"effectControls\"");
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Grading")});
            settings.setValue(QStringLiteral("workspace/saved/Grading"), old);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), old);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        // Editing's own changes from version 6 give way to the new
        // proportions (version 8: the 300 column over a 290 timeline), with
        // the Inspector open in the column; a workspace of one's own keeps
        // its arrangement, the Inspector renamed.
        QVERIFY(!workspace->panel(QStringLiteral("presets"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("inspector"))->isClosed());
        QCOMPARE(workspace->panel(QStringLiteral("inspector"))->dockAreaWidget(), workspace->panel(QStringLiteral("project"))->dockAreaWidget());
        workspace->resetCurrent();
        workspace->activate(QStringLiteral("Grading"));
        QCOMPARE(workspace->current(), QStringLiteral("Grading"));
        QVERIFY(workspace->panel(QStringLiteral("presets"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("inspector"))->isClosed());
        QCOMPARE(workspace->panel(QStringLiteral("inspector"))->dockAreaWidget(), workspace->panel(QStringLiteral("project"))->dockAreaWidget());
    }

    // At 1440x900 the Inspector fits its column: no row wider than the
    // column, and a video clip's identity, its primary rows and every
    // folded section header in view without scrolling.
    // Dragged all the way up, the timeline's split closes the row over it
    // altogether instead of stopping at a band of cut-off panels, and comes
    // back when dragged down; the timeline itself never closes. Short of
    // that, each dock area stops at its panels' least size, so nothing in
    // it is cut off, and the column of Project, Inspector, Effects and
    // Presets keeps one least width (no shorter than its tabs' names)
    // whichever is chosen.
    void timelineSplitClosesTheRowOverIt() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        ads::CDockAreaWidget *timeline = workspace->panel(QStringLiteral("timeline"))->dockAreaWidget();
        ads::CDockAreaWidget *browser = workspace->panel(QStringLiteral("project"))->dockAreaWidget();
        ads::CDockAreaWidget *viewer = workspace->panel(QStringLiteral("program"))->dockAreaWidget();
        auto *rows = qobject_cast<QSplitter *>(timeline->parentWidget());
        QVERIFY(rows && rows->orientation() == Qt::Vertical && rows->count() == 2);
        const int timelineIndex = rows->indexOf(timeline);
        const int topIndex = 1 - timelineIndex;
        QVERIFY(rows->isCollapsible(topIndex) && !rows->isCollapsible(timelineIndex));
        // Each area's least size is its panels' largest.
        for (const char *id : {"project", "inspector", "effects", "presets", "program", "source"}) {
            ads::CDockWidget *dock = workspace->panel(QString::fromLatin1(id));
            const QSize least = dock->dockAreaWidget()->minimumSizeHint();
            QVERIFY2(least.width() >= dock->widget()->minimumSizeHint().width() && least.height() >= dock->widget()->minimumSizeHint().height(),
                     id);
        }
        QVERIFY(browser->minimumSizeHint().width() >= browser->titleBar()->tabBar()->sizeHint().width());
        QVERIFY(timeline->minimumSizeHint().height() >= workspace->panel(QStringLiteral("timeline"))->widget()->minimumHeight());
        // The Editing arrangement holds the timeline's least height.
        QVERIFY(timeline->height() >= timeline->minimumSizeHint().height());
        // Choosing another tab leaves the column's width alone.
        const int browserWidth = browser->width();
        for (const char *id : {"inspector", "effects", "presets", "project"}) {
            workspace->panel(QString::fromLatin1(id))->setAsCurrentTab();
            QCoreApplication::processEvents();
            QCOMPARE(browser->width(), browserWidth);
        }
        // Part way up: the row stops at its least height, its panels whole.
        const QList<int> arranged = rows->sizes();
        const int total = arranged.at(0) + arranged.at(1);
        QList<int> partWay(2);
        partWay[topIndex] = 120;
        partWay[timelineIndex] = total - 120;
        rows->setSizes(partWay);
        QCoreApplication::processEvents();
        QVERIFY(viewer->height() >= viewer->minimumSizeHint().height());
        // All the way up, as a drag of its handle to the top: closed, the
        // timeline as tall as the rows.
        rows->setSizes(arranged);
        QCoreApplication::processEvents();
        QSplitterHandle *handle = rows->handle(1);
        const QPoint grip(handle->width() / 2, handle->height() / 2);
        QTest::mousePress(handle, Qt::LeftButton, Qt::NoModifier, grip);
        QTest::mouseMove(handle, grip - QPoint(0, handle->mapTo(rows, QPoint()).y() + 40));
        QTest::mouseRelease(handle, Qt::LeftButton, Qt::NoModifier, grip - QPoint(0, handle->mapTo(rows, QPoint()).y() + 40));
        QCoreApplication::processEvents();
        QCOMPARE(rows->sizes().at(topIndex), 0);
        QCOMPARE(rows->sizes().at(timelineIndex), total);
        // Dragged down again, the row comes back.
        rows->setSizes(arranged);
        QCoreApplication::processEvents();
        QVERIFY(viewer->height() > 0 && viewer->isVisible());
        // As it does from a restored layout.
        workspace->resetCurrent();
        QCoreApplication::processEvents();
        rows = qobject_cast<QSplitter *>(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->parentWidget());
        QVERIFY(rows && rows->childrenCollapsible());
        QVERIFY(!rows->isCollapsible(rows->indexOf(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget())));
    }

    // A clip of a shape seen a moment ago shows the same rows again, with
    // its own values: going back and forth between a title and a video
    // clip leaves one body in the column, never a stale one.
    void inspectorShowsAShapeAgainWithItsValues() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        InspectorPanel *inspector = window.m_inspector;
        const auto visibleRows = [inspector] {
            QStringList labels;
            for (InspectorRow *row : inspector->findChildren<InspectorRow *>())
                if (row->isVisibleTo(inspector)) labels << row->label()->text();
            return labels;
        };
        window.clickSelectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        const QStringList titleRows = visibleRows();
        const QString titleName = inspector->nameText();
        QVERIFY(!titleRows.isEmpty());
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        const QStringList videoRows = visibleRows();
        QVERIFY(videoRows != titleRows);
        for (int round = 0; round < 2; ++round) {
            window.clickSelectClip(QStringLiteral("t"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), titleRows);
            QCOMPARE(inspector->nameText(), titleName);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
            window.clickSelectClip(QStringLiteral("v"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), videoRows);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
        }
        // A new appearance builds every body again.
        const ColorScheme was = editorColorScheme();
        const auto back = qScopeGuard([was] { setEditorColorScheme(was); });
        setEditorColorScheme(was == ColorScheme::Dark ? ColorScheme::Light : ColorScheme::Dark);
        window.clickSelectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        QCOMPARE(visibleRows(), titleRows);
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
    }

    void inspectorFitsItsColumn() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->isVisible());
        const auto unclipped = [inspector](const QString &what) {
            for (InspectorRow *row : inspector->findChildren<InspectorRow *>()) {
                if (!row->isVisibleTo(inspector)) continue;
                for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly)) {
                    if (!child->isVisibleTo(row)) continue;
                    QVERIFY2(child->geometry().right() < row->width() && child->x() >= 0,
                             qPrintable(QStringLiteral("%1: %2 in %3").arg(what, child->objectName(), row->label()->text())));
                }
                // Every value whole: "+0.40 EV", "−8.0 dB", "−48 px".
                for (ScrubField *field : row->findChildren<ScrubField *>())
                    if (field->isVisibleTo(row))
                        QVERIFY2(field->showsWhole(), qPrintable(QStringLiteral("%1: %2 %3").arg(what, row->label()->text(), field->text())));
                QVERIFY2(row->geometry().right() < row->parentWidget()->width(), qPrintable(row->label()->text()));
            }
        };
        unclipped(QStringLiteral("column"));
        // Start, End and Duration share the column in thirds, and the kind
        // line runs to the column's edge (under ⋯), so neither is cut short
        // at 300 points (text widths differ by platform; the room does not).
        QCOMPARE(inspector->m_timingFields.size(), 3);
        for (ScrubField *field : inspector->m_timingFields)
            QVERIFY2(field->width() >= (inspector->width() - 2 * 16 - 2 * 8) / 3 - 1, qPrintable(QString::number(field->width())));
        QVERIFY(inspector->m_kind->x() + inspector->m_kind->width() >= inspector->width() - 16 - 1);
        // Everything of the video, folded, is in view.
        QScrollBar *scroll = inspector->scrollArea()->verticalScrollBar();
        QVERIFY2(scroll->maximum() == 0, qPrintable(QStringLiteral("scrolls %1 in %2").arg(scroll->maximum()).arg(inspector->height())));
        const QRect viewport(inspector->scrollArea()->viewport()->mapTo(inspector, QPoint()), inspector->scrollArea()->viewport()->size());
        for (const char *section : {"crop", "anchor", "linkedAudio", "masks", "effects", "source"}) {
            InspectorSection *found = inspector->section(QString::fromLatin1(section));
            QVERIFY2(found, section);
            const QRect header(found->header()->mapTo(inspector, QPoint()), found->header()->size());
            QVERIFY2(viewport.contains(header), section);
        }
        // The same in a 16:9 project (the laptop): one stage row for every
        // shape, so the column is as tall and nothing scrolls either.
        {
            QJsonObject wide = everyKindSnapshot(2);
            QJsonObject timeline = wide.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1920);
            timeline.insert(QStringLiteral("height"), 1080);
            wide.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(wide);
            QCoreApplication::processEvents();
            QCOMPARE(window.m_viewer->device(), ViewerWidget::Device::Laptop);
            QVERIFY2(scroll->maximum() == 0, qPrintable(QStringLiteral("16:9 scrolls %1 in %2").arg(scroll->maximum()).arg(inspector->height())));
            InspectorSection *source = inspector->section(QStringLiteral("source"));
            QVERIFY(source);
            QVERIFY(viewport.contains(QRect(source->header()->mapTo(inspector, QPoint()), source->header()->size())));
            unclipped(QStringLiteral("16:9 column"));
        }
        // Every row lines its controls up after one label column, and ends
        // at one edge: the reset lane's, 24 points inside the column's.
        int controlsRight = -1;
        for (InspectorRow *row : inspector->findChildren<InspectorRow *>()) {
            QCOMPARE(row->label()->width(), InspectorRow::kLabelWidth);
            QCOMPARE(row->height(), InspectorRow::kHeight);
            if (!row->isVisibleTo(inspector)) continue;
            int right = -1;
            for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly))
                if (child != row->label() && child->isVisibleTo(row) && child->width() > InspectorRow::kResetLane &&
                    child->sizePolicy().horizontalPolicy() != QSizePolicy::Fixed)
                    right = std::max(right, child->geometry().right());
            // One value field is the column's one width; two (X and Y, L and
            // R) share the row equally, 78 + 8 + 78 at a 300-point column.
            QList<ScrubField *> fields;
            for (ScrubField *field : row->findChildren<ScrubField *>(QString(), Qt::FindDirectChildrenOnly))
                if (field->isVisibleTo(row)) fields << field;
            if (fields.size() == 2) {
                QCOMPARE(fields.at(0)->width(), fields.at(1)->width());
                QVERIFY(fields.at(0)->width() >= InspectorRow::kValueField);
            } else {
                for (ScrubField *field : fields) QCOMPARE(field->width(), InspectorRow::kValueField);
            }
            if (right < 0) continue;
            if (controlsRight < 0) controlsRight = right;
            QVERIFY2(right <= controlsRight, qPrintable(row->label()->text()));
        }
        QVERIFY(controlsRight > 0);
        QVERIFY(controlsRight <= inspector->width() - 16 - 24 + 1);
        // At the narrower 300-point column, still nothing is cut off.
        auto *alone = new InspectorPanel;
        alone->setEffectCatalog(QJsonArray{});
        alone->setMaskable(true);
        alone->setMaskCatalog(QJsonArray{QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 10000}, {"unit", "pixels"}}},
                              {QStringLiteral("add")});
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        alone->setState(timeline, {QStringLiteral("v")});
        alone->resize(300, 560);
        alone->show();
        QVERIFY(QTest::qWaitForWindowExposed(alone));
        for (InspectorSection *section : alone->findChildren<InspectorSection *>()) section->setOpen(true);
        QCoreApplication::processEvents();
        const auto check = [alone](const QString &what) {
            for (InspectorRow *row : alone->findChildren<InspectorRow *>()) {
                if (!row->isVisibleTo(alone)) continue;
                for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly)) {
                    if (!child->isVisibleTo(row)) continue;
                    QVERIFY2(child->geometry().right() < row->width(), qPrintable(QStringLiteral("%1: %2").arg(what, row->label()->text())));
                }
                QVERIFY2(row->mapTo(alone, QPoint(row->width(), 0)).x() <= alone->width() - 16,
                         qPrintable(QStringLiteral("%1 %2: right %3, min %4").arg(what, row->label()->text())
                                        .arg(row->mapTo(alone, QPoint(row->width(), 0)).x()).arg(row->minimumSizeHint().width())));
            }
        };
        check(QStringLiteral("300"));
        // Nothing selected: the frame size reads whole in its pop-up, clear
        // of its ⌃⌄.
        alone->setState(timeline, {});
        QCoreApplication::processEvents();
        {
            auto *size = alone->findChild<QComboBox *>(QStringLiteral("frameSize"));
            QVERIFY(size && size->isVisible());
            QVERIFY2(QFontMetrics(size->font()).horizontalAdvance(size->currentText()) + 28 <= size->width(),
                     qPrintable(QStringLiteral("%1 in %2").arg(size->currentText()).arg(size->width())));
        }
        for (const char *id : {"t", "music", "adj"}) {
            alone->setState(timeline, {QString::fromLatin1(id)});
            for (InspectorSection *section : alone->findChildren<InspectorSection *>()) section->setOpen(true);
            QCoreApplication::processEvents();
            check(QString::fromLatin1(id));
        }
        delete alone;
    }

    // The Inspector for every kind of clip, nothing selected and several
    // clips, in both appearances, and the whole editor with a clip clicked
    // in 9:16 and 16:9. With KERF_SCREENSHOT_DIR set, each is saved as
    // inspector-<state>-<scheme>.png and editor-<format>-inspector-<scheme>.png.
    void inspectorRendersEveryState() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        // The showcase with the close-up graded, scaled up and masked, and
        // more kinds of clip.
        const auto snapshotFor = [](const QSize &canvas) {
            QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
            snapshot.remove(QStringLiteral("pendingProposal"));
            snapshot.remove(QStringLiteral("proposalTimeline"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), canvas.width());
            timeline.insert(QStringLiteral("height"), canvas.height());
            timeline.insert(QStringLiteral("sampleRate"), 48000);
            timeline.insert(QStringLiteral("properties"), catalogProperties());
            timeline.insert(QStringLiteral("generators"), catalogGenerators());
            timeline.insert(QStringLiteral("maskModes"), QJsonArray{"add", "subtract", "intersect", "none"});
            timeline.insert(QStringLiteral("maskProperties"), QJsonArray{
                QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0.0}, {"min", 0.0}, {"max", 10000.0}, {"unit", "pixels"}},
                QJsonObject{{"name", "opacity"}, {"type", "number"}, {"default", 1.0}, {"min", 0.0}, {"max", 1.0}},
                QJsonObject{{"name", "expansion"}, {"type", "number"}, {"default", 0.0}, {"min", -10000.0}, {"max", 10000.0}, {"unit", "pixels"}}});
            QJsonArray tracks = timeline.value(QStringLiteral("tracks")).toArray();
            const auto edit = [&tracks](int track, int index, const std::function<void(QJsonObject &)> &change) {
                QJsonObject t = tracks.at(track).toObject();
                QJsonArray clips = t.value(QStringLiteral("clips")).toArray();
                QJsonObject clip = clips.at(index).toObject();
                change(clip);
                clips.replace(index, clip);
                t.insert(QStringLiteral("clips"), clips);
                tracks.replace(track, t);
            };
            edit(0, 1, [](QJsonObject &clip) {
                clip.insert("startFrame", 144);
                clip.insert("durationFrames", 120);
                clip.insert("speed", 1.0);
                clip.insert("link", "L2");
                clip.insert("props", QJsonObject{{"scale", 1.12}, {"y", -0.025}, {"crop_b", 0.0556}});
                clip.insert("masks", QJsonArray{QJsonObject{{"name", "vignette"}, {"ellipse", QJsonArray{0.5, 0.5, 0.4, 0.4}}, {"mode", "add"},
                                                            {"inverted", false}, {"params", QJsonObject{{"feather", 40.0}}}}});
                clip.insert("effects", QJsonArray{
                    QJsonObject{{"fx", "color.basic"}, {"enabled", true},
                                {"params", QJsonObject{{"exposure", 0.4}, {"contrast", 1.1}, {"saturation", 0.92}, {"temperature", 350.0}}}},
                    QJsonObject{{"fx", "blur.gaussian"}, {"enabled", false}, {"params", QJsonObject{{"radius", 8.0}}}}});
            });
            edit(1, 0, [](QJsonObject &clip) {
                clip.insert("props", QJsonObject{{"text", "Harbor Teaser"}, {"size", 0.06}, {"y", 0.28}});
            });
            QJsonObject titles = tracks.at(1).toObject();
            QJsonArray titleClips = titles.value(QStringLiteral("clips")).toArray();
            titleClips.append(QJsonObject{{"id", "adj"}, {"kind", "generator"}, {"fx", "adjustment"}, {"name", "Warm grade"},
                                          {"timelineStart", 6.0}, {"duration", 5.0},
                                          {"effects", QJsonArray{QJsonObject{{"fx", "color.basic"}, {"enabled", true},
                                                                             {"params", QJsonObject{{"temperature", 400.0}, {"saturation", 1.1}}}}}}});
            titles.insert(QStringLiteral("clips"), titleClips);
            tracks.replace(1, titles);
            edit(3, 1, [](QJsonObject &clip) { clip.insert("link", "L2"); clip.insert("speed", 1.0); });
            edit(4, 0, [](QJsonObject &clip) {
                clip.insert("speed", 1.0);
                clip.insert("props", QJsonObject{{"gain", -8.0}, {"pan", -0.4}});
            });
            timeline.insert(QStringLiteral("tracks"), tracks);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            return snapshot;
        };
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QString name = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");
            for (const QSize canvas : {QSize(1080, 1920), QSize(1920, 1080)}) {
                const bool tall = canvas.height() > canvas.width();
                QSettings().remove(QStringLiteral("workspace"));
                QSettings().remove(QStringLiteral("editor/inspector"));
                MainWindow window{QString(), QString()};
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshotFor(canvas));
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                // The playhead in the close-up, where the frame shown is: the
                // time on the picture and the timeline's playhead agree with it.
                window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 6.0}, {"duration", 18.0}});
                window.m_viewer->setFrame(testing::footageFrame(testing::Shot::FisherClose, canvas / 4), 6.0);
                QCOMPARE(window.m_timeline->m_playhead, 6.0);
                window.m_inspector->setProgramFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(160, 90)));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Warm up the close-up a little."));
                window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I raised the exposure and the temperature on fisher-close.mov."));
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                window.clickSelectClip(QStringLiteral("v1b"));
                QTest::qWait(30);
                ads::CDockWidget *panel = window.m_workspace->panel(QStringLiteral("inspector"));
                QVERIFY(panel->isCurrentTab());
                InspectorPanel *inspector = window.m_inspector;
                // The chat is untouched beside the picture.
                QVERIFY(window.m_chat->isVisible());
                QCOMPARE(window.m_chat->geometry(), window.m_viewer->stageLayout().panel);
                QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf about fisher-close.mov…"));
                const QImage editor = window.grab().toImage();
                QCOMPARE(amberPixels(editor), 0);
                save(editor, QStringLiteral("editor-%1-inspector-%2").arg(tall ? QStringLiteral("9x16") : QStringLiteral("16x9"), name));
                if (!tall) continue;
                // The panel's own surface, a hairline above each folded
                // header, and fields filled in the Inspector's field colour.
                const auto grabPanel = [panel] { return panel->grab().toImage(); };
                {
                    const QImage image = grabPanel();
                    const qreal ratio = image.devicePixelRatio();
                    InspectorSection *crop = inspector->section(QStringLiteral("crop"));
                    const QPoint rule = crop->header()->mapTo(panel, QPoint(crop->header()->width() / 2, 0));
                    QCOMPARE(image.pixelColor(rule * ratio).rgb(), theme.divider.rgb());
                    QCOMPARE(image.pixelColor((rule + QPoint(0, 2)) * ratio).rgb(), theme.panel.rgb());
                    ScrubField *scale = inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"));
                    const QPoint inside = scale->mapTo(panel, QPoint(3, scale->height() / 2));
                    QCOMPARE(image.pixelColor(inside * ratio).rgb(), theme.inspectorField.rgb());
                    // No purple anywhere in the Inspector at rest.
                    QCOMPARE(dominantColor(image), theme.panel.rgb());
                }
                // The video, its Scale row under the pointer: ↺ and its changed value.
                auto *scaleRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"))->parentWidget());
                {
                    QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), QPointF(4, 4));
                    QCoreApplication::sendEvent(scaleRow, &enter);
                    QCoreApplication::sendEvent(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale")), &enter);
                }
                QVERIFY(scaleRow->resetButton()->isVisible());
                save(grabPanel(), QStringLiteral("inspector-video-%1").arg(name));
                {
                    QEvent leave(QEvent::Leave);
                    QCoreApplication::sendEvent(scaleRow, &leave);
                    QCoreApplication::sendEvent(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale")), &leave);
                }
                // Folded, the Effects header in the 300pt column names what
                // is on in whole words, never cut ("…"), and counts the rest.
                {
                    InspectorSection *fx = inspector->section(QStringLiteral("effects"));
                    QVERIFY(!fx->isOpen());
                    QCoreApplication::processEvents();
                    auto *label = fx->findChild<QLabel *>(QStringLiteral("sectionSummary"));
                    const QString shown = fx->shownSummary();
                    QVERIFY2(!shown.isEmpty() && !shown.contains(QChar(0x2026)), qPrintable(shown));
                    QVERIFY2(label->fontMetrics().horizontalAdvance(shown) <= label->contentsRect().width(), qPrintable(shown));
                    QVERIFY(fx->summary().startsWith(QStringLiteral("Primary Colour Correction")));
                    QCOMPARE(label->toolTip(), fx->summary());
                }
                // Its effects: Color open with "6 more", the blur folded with its line.
                inspector->section(QStringLiteral("effects"))->setOpen(true);
                for (const char *section : {"crop", "anchor", "linkedAudio", "masks", "source"})
                    inspector->section(QString::fromLatin1(section))->setOpen(false);
                const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
                QCOMPARE(boxes.size(), 2);
                boxes.at(0)->findChild<GlyphButton *>(QStringLiteral("effectChevron"))->click();
                QCOMPARE(boxes.at(0)->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"))->text(), QStringLiteral("6 more"));
                QCOMPARE(boxes.at(1)->findChild<QLabel *>(QStringLiteral("effectSummary"))->text(), QStringLiteral("Radius 8 px"));
                QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("effectParam_exposure"))->text(), QStringLiteral("+0.40"));
                QCoreApplication::processEvents();
                for (ScrubField *field : boxes.at(0)->findChildren<ScrubField *>())
                    if (field->isVisible()) QVERIFY2(field->showsWhole(), qPrintable(field->objectName()));
                QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("effectParam_contrast"))->text(), QStringLiteral("110"));
                QCoreApplication::processEvents();
                inspector->scrollArea()->ensureWidgetVisible(boxes.at(0), 0, 0);
                inspector->scrollArea()->verticalScrollBar()->setValue(inspector->section(QStringLiteral("effects"))->y() - 8);
                QCoreApplication::processEvents();
                save(grabPanel(), QStringLiteral("inspector-video-effects-%1").arg(name));
                inspector->section(QStringLiteral("effects"))->setOpen(false);
                // The other kinds.
                for (const auto &[clip, label] : {std::pair{"a2", "audio"}, std::pair{"t1", "title"}, std::pair{"adj", "adjustment"}, std::pair{"c1", "caption"}}) {
                    window.clickSelectClip(QString::fromLatin1(clip));
                    QCoreApplication::processEvents();
                    QVERIFY2(panel->isCurrentTab(), qPrintable(QStringLiteral("%1: %2").arg(QString::fromLatin1(clip),
                                                                 panel->dockAreaWidget()->currentDockWidget()->objectName())));
                    save(grabPanel(), QStringLiteral("inspector-%1-%2").arg(QString::fromLatin1(label), name));
                }
                // Nothing selected (with the Inspector's tab chosen by hand, so it stays).
                window.m_workspace->panel(QStringLiteral("project"))->setAsCurrentTab();
                panel->setAsCurrentTab();
                window.clickSelectClip(QString());
                QCoreApplication::processEvents();
                QVERIFY(panel->isCurrentTab());
                save(grabPanel(), QStringLiteral("inspector-nothing-%1").arg(name));
                // Several clips.
                inspector->setSelection({QStringLiteral("v1a"), QStringLiteral("v1b"), QStringLiteral("v1c")});
                QCoreApplication::processEvents();
                save(grabPanel(), QStringLiteral("inspector-multi-%1").arg(name));
                window.close();
            }
        }
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
        window.setChatOpen(false);
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
        window.setChatOpen(false);
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
        QCOMPARE(image.pixelColor(beat.center().x(), beat.top() + 2), editorTheme().marker(QStringLiteral("red")));
        QCOMPARE(image.pixelColor(chorus.left() + 4, chorus.top() + 2), editorTheme().marker(QString()));
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
        // Tracks start dense (eight fit the timeline at 1440x900): one-line
        // headers, a filmstrip under a picture clip's name and a waveform
        // under a sound clip's; Mini is one line of 18px.
        QVERIFY(kVideoTrackHeight < kDetailedTrackHeight && kVideoTrackHeight >= 28);
        QVERIFY(kTrackHeight <= kVideoTrackHeight && kTrackHeight >= 22);
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
        QCOMPARE(dock->titleBarActions(), window.m_timeline->trackHeightActions());
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
        QCOMPARE(trackHeights(*timeline), heights(30, 26));
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        QTest::keyClick(timeline, Qt::Key_Down);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));

        const auto wheel = [timeline](const QPoint &at, int delta, Qt::KeyboardModifiers modifiers,
                                      Qt::ScrollPhase phase = Qt::NoScrollPhase) {
            QWheelEvent event(at, timeline->mapToGlobal(at), QPoint(), QPoint(0, delta), Qt::NoButton, modifiers, phase, false);
            QApplication::sendEvent(timeline, &event);
            return event.isAccepted();
        };
        const auto inView = [timeline, &scroll](const QPoint &point) { return timeline->mapFrom(scroll.viewport(), point); };
        QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        // A trackpad's small deltas add up to a notch.
        for (int part = 0; part < 3; ++part) wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        // Without Option, or over the lanes, the wheel is the scroll area's.
        QVERIFY(!wheel(inView(QPoint(120, 200)), 120, Qt::NoModifier));
        QVERIFY(!wheel(inView(QPoint(500, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        // The momentum after a flick steps nothing, and a new gesture starts
        // its notch afresh.
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollBegin));
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollUpdate));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        for (int event = 0; event < 6; ++event) QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier, Qt::ScrollMomentum));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollEnd);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollBegin);
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));

        // The track under the pointer stays where it is in the view.
        scroll.verticalScrollBar()->setValue(300);
        QCoreApplication::processEvents();
        const QPoint over = inView(QPoint(120, 250));
        const int row = timeline->trackRowAt(over.y());
        QVERIFY(row > 5);
        const auto top = [timeline](int at) { return timeline->trackHeaderRect(at).top() + timeline->pos().y(); };
        const int before = top(row);
        wheel(over, -120, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(18, 18));
        QCOMPARE(top(row), before);
        // (Over the same track, now shorter.)
        wheel(QPoint(over.x(), timeline->trackHeaderRect(row).top() + 2), 240, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(26, 26));
        QCOMPARE(top(row), before);
        // The keys keep the track at the top of the view in place.
        const int first = timeline->trackRowAt(-timeline->pos().y() + kRulerHeight);
        const int firstTop = top(first);
        QTest::keyClick(timeline, Qt::Key_Up, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(30, 30));
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
        QCOMPARE(taller, (QList<int>{128, 104, 104, 26, 26}));
        timeline.stepAllTrackHeights(-1);
        QCOMPARE(trackHeights(timeline), (QList<int>{104, 86, 86, 22, 22}));
        // The tallest stays the tallest; Option-scroll steps the same way.
        QWheelEvent wheel(QPointF(120, timeline.trackHeaderRect(3).center().y()), timeline.mapToGlobal(QPointF(120, 0)), QPoint(),
                          QPoint(0, 120), Qt::NoButton, Qt::AltModifier, Qt::NoScrollPhase, false);
        QApplication::sendEvent(&timeline, &wheel);
        QCOMPARE(trackHeights(timeline), (QList<int>{128, 104, 104, 26, 26}));
        // A new track takes the step every track without a height of its
        // own was taken to: one step from the picture default.
        QJsonArray tracks = project.value("tracks").toArray();
        tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
        project.insert("tracks", tracks);
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.allTrackHeight(), 30);
        QCOMPARE(trackHeights(timeline).constLast(), 30);
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
                // No plates: the number is bare text and the toggles bare
                // symbols on the header (a wash only under the pointer).
                // (Inside where a plate's rounded corner was, clear of the symbols.)
                if (output.height() >= 16)
                    for (const QRect &control : {output, lock})
                        QCOMPARE(image.pixelColor(control.left() + 2, control.top() + 1), theme.trackHeader);
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
            QCOMPARE(recorder.colorOf(QStringLiteral("V1")), theme.trackBadgeText);
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
            // A paint still pending at the grab shows as a change here.
            QCoreApplication::processEvents();
            const bool settled = scrolled == onScreen();
            scroll.viewport()->repaint();
            const QImage repainted = onScreen();
            // Where they differ, to tell a stale strip from a stray pixel.
            QRect differ;
            int count = 0;
            if (scrolled.size() == repainted.size())
                for (int y = 0; y < scrolled.height(); ++y)
                    for (int x = 0; x < scrolled.width(); ++x)
                        if (scrolled.pixel(x, y) != repainted.pixel(x, y)) {
                            differ |= QRect(x, y, 1, 1);
                            ++count;
                        }
            QVERIFY2(scrolled == repainted,
                     qPrintable(QStringLiteral("after %1, %2: %3 px differ in %4,%5 %6x%7 (%8 px painted; viewport at %9,%10 %11x%12; %13)")
                                    .arg(step.dx)
                                    .arg(step.dy)
                                    .arg(count)
                                    .arg(differ.x())
                                    .arg(differ.y())
                                    .arg(differ.width())
                                    .arg(differ.height())
                                    .arg(painted)
                                    .arg(scroll.viewport()->x())
                                    .arg(scroll.viewport()->y())
                                    .arg(view.width())
                                    .arg(view.height())
                                    .arg(settled ? QStringLiteral("settled") : QStringLiteral("changed on a second pass"))));
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
        const auto conversation = window.m_chat->transcript();
        const int activityCount = window.m_activityList->count();
        window.showError(QStringLiteral("Render failed"));
        window.showError(QStringLiteral("Render failed"));
        QCOMPARE(window.m_chat->transcript(), conversation);
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
                // A chosen recent project is the selection while its list has focus.
                if (page == ProjectLauncher::Recent) launcher.m_recent->setFocus();
                QCoreApplication::processEvents();
                QImage image = launcher.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const auto at = [&image, ratio](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio).rgb(); };
                QCOMPARE(at(launcher.width() - 6, 6), launcher.theme().content.rgb());
                QCOMPARE(at(100, launcher.height() - 10), launcher.theme().sidebar.rgb());
                // Antique cream in light, warm espresso in dark: never pure
                // white or black, and warm (red above blue).
                QVERIFY(isWarmPaper(launcher.theme().content, dark));
                QVERIFY(isWarmPaper(launcher.theme().sidebar, dark));
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
                    // launcher's secondary text colour, and dims when the field is off.
                    const LauncherTheme &look = launcher.theme();
                    launcher.m_remote->ensurePolished();
                    QVERIFY(launcher.m_remote->isEnabled() && launcher.m_remote->text().isEmpty());
                    for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive})
                        QCOMPARE(launcher.m_remote->palette().color(group, QPalette::PlaceholderText).rgb(),
                                 look.secondary.rgb());
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
    // without focus, and the details on the focused list's selection. The colours
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

    // Whether `color` is one of the palette's paper surfaces: antique cream
    // in light (light, but not white) and espresso in dark (dark, but not
    // black), warm in both (red above blue).
    static bool isWarmPaper(const QColor &color, bool dark) {
        const bool warm = color.red() >= color.blue() + (dark ? 4 : 10);
        return warm && (dark ? color.lightness() < 45 && color.rgb() != QColor(Qt::black).rgb()
                             : color.lightness() > 200 && color.rgb() != QColor(Qt::white).rgb());
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

    static void verifyListEnd(ChatPanel *chat) {
        QScrollArea *scroll = chat->scroll();
        QScrollBar *bar = scroll->verticalScrollBar();
        QTRY_COMPARE(bar->value(), bar->maximum());
        QCoreApplication::processEvents();
        QWidget *content = scroll->widget();
        int last = 0;
        for (QWidget *child : content->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
            if (child->isVisible()) last = std::max(last, child->geometry().bottom() + 1);
        const int top = bar->value();
        const int bottom = top + scroll->viewport()->height();
        if (bar->maximum() > 0)
            QVERIFY2(std::abs(last - bottom) <= 2, qPrintable(QStringLiteral("%1 px under the latest").arg(bottom - last)));
        else
            QVERIFY(last <= bottom);
        // The view itself stops 16 points over the chips (or the composer),
        // so the rows are cut there at every scroll position, not only at
        // the list's end.
        QWidget *under = chat->chips()->isVisible() ? chat->chips() : static_cast<QWidget *>(chat->composer());
        QCOMPARE(under->mapTo(chat, QPoint()).y() - (scroll->viewport()->mapTo(chat, QPoint()).y() + scroll->viewport()->height()),
                 ChatPanel::kListBottom);
        // Nothing over the view's top: the rows run right up to its edge.
        for (QWidget *over : scroll->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
            QVERIFY2(over == scroll->viewport() || over->objectName().startsWith(QStringLiteral("qt_scrollarea")) || !over->isVisible(),
                     qPrintable(over->objectName()));
        QWidget *edge = chat->listEdge();
        const int edgeTop = edge->mapTo(chat, QPoint()).y();
        QCOMPARE(scroll->viewport()->mapTo(chat, QPoint()).y(), edgeTop + edge->height());
        QCOMPARE(edgeTop, chat->proposalBar()->isVisible() ? chat->proposalBar()->geometry().bottom() + 1 : ChatPanel::kHeaderHeight);
        QCOMPARE(edge->property("scrolled").toBool(), top > 0);
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
    // Whether two colours match to within `tolerance` per channel: a
    // translucent fill laid on its surface by the painter rounds its own way.
    static bool nearColor(QRgb a, QRgb b, int tolerance = 2) {
        return std::abs(qRed(a) - qRed(b)) <= tolerance && std::abs(qGreen(a) - qGreen(b)) <= tolerance &&
               std::abs(qBlue(a) - qBlue(b)) <= tolerance;
    }

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
                                       QJsonObject{{"id", "m2"}, {"time", 13.0}, {"duration", 2.0}, {"color", "green"}}}},
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
    // without FFmpeg: frames of the tests' procedural footage (see
    // test_footage.h), one shot per source, made once per run.
    static void fakeArtwork(MediaArtwork *artwork) {
        static const QImage wide = testing::footageFrame(testing::Shot::HarborWide, QSize(160, 90));
        static const QImage close = testing::footageFrame(testing::Shot::FisherClose, QSize(160, 90));
        static const QImage drone = testing::footageFrame(testing::Shot::DronePass, QSize(160, 90));
        artwork->frames.insert(QStringLiteral("wide"), wide);
        artwork->frames.insert(QStringLiteral("close"), close);
        artwork->frames.insert(QStringLiteral("drone"), drone);
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

    // Kerf has its own accent, a purple in each appearance, and does not
    // follow the system's: the editor and the launcher keep it when the
    // platform reports another accent, and switch only with light and dark.
    void kerfDrawsItsOwnPurpleAccent() {
        const auto restore = qScopeGuard([] {
            QGuiApplication::setPalette(QPalette());
            setEditorColorScheme(ColorScheme::Dark);
        });
        for (const bool dark : {false, true}) {
            const EditorTheme t = makeEditorTheme(dark);
            const LauncherTheme l = launcherTheme(dark);
            QCOMPARE(t.accent.rgb(), kerfAccent(dark).rgb());
            QCOMPARE(l.accent.rgb(), t.accent.rgb());
            // Purple: between blue and red, and clearly coloured.
            QVERIFY2(t.accent.hslHue() >= 270 && t.accent.hslHue() <= 320, qPrintable(t.accent.name()));
            QVERIFY2(t.accent.hslSaturation() >= 40, qPrintable(t.accent.name()));
            // Kerf's own marks are the accent: the playhead, focus, the
            // agent's proposals and its avatar.
            for (const QColor &mark : {t.playhead, t.focusRing, t.assistant, t.proposalAdd, t.chatAvatarKerf, t.maskHandleFill})
                QCOMPARE(mark.rgb(), t.accent.rgb());
            // Track toggles are bare symbols: on, a neutral ink, not the
            // accent; off, the same ink, faint. No plate under either.
            QVERIFY(t.trackToggleOn.rgb() != t.accent.rgb() && t.trackToggleOn.hsvSaturation() < 90);
            QCOMPARE(t.trackToggle.rgb(), t.trackToggleOn.rgb());
            QVERIFY(t.trackToggle.alpha() <= t.trackToggleOn.alpha() / 2);
            // In dark the mauve carries dark ink, so the launcher's chosen
            // rows (a deep plum) and its switch knob have their own tokens.
            QVERIFY(contrastRatio(l.selectionText, l.selection) >= 4.5);
            if (dark) QVERIFY(l.selectionText.rgb() != l.onAccent.rgb());
            // Fewer boxes: the caption on the picture has no line round it,
            // the open chat's toggle is a tonal disc with Kerf's ink, and the
            // launcher's format tiles are paper cards lighter than the page.
            QCOMPARE(t.viewerCaptionBorder.rgba(), t.viewerCaption.rgba());
            QCOMPARE(t.railButtonActive.rgba(), t.railButton.rgba());
            QCOMPARE(t.railGlyphActive.rgb(), t.accentText.rgb());
            QVERIFY(l.tile.lightness() > l.content.lightness());
            // No blue on the timeline: no clip kind's header is a blue
            // (hue 190-250 with any real colour in it), and picture clips,
            // the largest coloured area, are a dull heather well under the
            // accent's saturation.
            for (const ClipStyle *style : {&t.clipVideo, &t.clipAudio, &t.clipMusic, &t.clipGenerator, &t.clipCaption}) {
                const QColor header = style->header;
                const bool blue = header.hslHueF() * 360 >= 190 && header.hslHueF() * 360 <= 250 && header.hslSaturationF() > 0.15;
                QVERIFY2(!blue, qPrintable(header.name()));
            }
            QVERIFY(t.clipVideo.header.hslSaturationF() < t.accent.hslSaturationF() / 2);
            QVERIFY(t.clipVideo.header.rgb() != t.clipCaption.header.rgb());
            QVERIFY(contrastRatio(t.clipVideo.label, t.clipVideo.header) >= 4.5);
        }
        // The light and dark accents differ: ink on cream, mauve on espresso.
        QVERIFY(kerfAccent(true).lightness() > kerfAccent(false).lightness() + 60);
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
        // A system accent of any colour leaves Kerf's in place.
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            QPalette blue = QGuiApplication::palette();
            blue.setColor(QPalette::Accent, QColor("#007aff"));
            QGuiApplication::setPalette(blue);
            applyEditorAppearance();
            const bool dark = scheme == ColorScheme::Dark;
            QCOMPARE(editorTheme().accent.rgb(), kerfAccent(dark).rgb());
            ProjectLauncher launcher;
            launcher.setColorScheme(dark ? ProjectLauncher::ColorScheme::Dark : ProjectLauncher::ColorScheme::Light);
            QCOMPARE(launcher.theme().accent.rgb(), kerfAccent(dark).rgb());
            QGuiApplication::setPalette(QPalette());
        }
#endif
    }

    // The accent as text (the transports' timecodes, the timeline's corner
    // time, the sync chip) reads at 4.5:1 on each surface it is drawn on, in
    // both appearances; text on the filled accent reads at 4.5:1 too, with
    // no exemption.
    void accentTextReadsOnEverySurfaceItIsDrawnOn() {
        for (const bool dark : {false, true}) {
            const EditorTheme theme = makeEditorTheme(dark);
            for (const QColor &surface : {theme.panel, theme.rulerCorner, theme.statusBar, theme.raised, theme.window,
                                          theme.chrome, theme.well, theme.field, theme.dialog, theme.group, theme.chatSurface}) {
                const qreal ratio = contrastRatio(theme.accentText, surface);
                QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1 on %2 in %3: %4")
                                                      .arg(theme.accentText.name(), surface.name(),
                                                           dark ? QStringLiteral("dark") : QStringLiteral("light"))
                                                      .arg(ratio, 0, 'f', 2)));
            }
            // So does the Source monitor's marked range, at 3:1 on its panel.
            QVERIFY(contrastRatio(theme.inOutRange, theme.panel) >= 3.0);
            // Text on the accent's fill, at rest, lit and held.
            for (const QColor &fill : {theme.accent, theme.accentTop, theme.accentPressed})
                QVERIFY2(contrastRatio(theme.onAccent, fill) >= 4.5, qPrintable(fill.name()));
        }
    }

    // The chrome keeps one type scale: every size the editor's, the docks'
    // and the launcher's style sheets set, and every size the timeline
    // paints in, is 11, 12, 13, 14, 16 or 20px. (The chat's composer is
    // rebuilt separately and keeps its own for now.)
    void chromeKeepsOneTypeScale() {
        const QSet<int> scale{11, 12, 13, 14, 16, 20};
        static const QRegularExpression rule(QStringLiteral("([^{}]+)\\{([^{}]*)\\}"));
        static const QRegularExpression size(QStringLiteral("font-size:\\s*(\\d+)px"));
        for (const bool dark : {false, true}) {
            const EditorTheme t = makeEditorTheme(dark);
            for (const QString &sheet : {editorStyleSheet(t), dockStyleSheet(t), launcherStyleSheet(launcherTheme(dark))}) {
                for (auto it = rule.globalMatch(sheet); it.hasNext();) {
                    const QRegularExpressionMatch match = it.next();
                    const QString selector = match.captured(1).trimmed();
                    for (auto sizes = size.globalMatch(match.captured(2)); sizes.hasNext();) {
                        const int pixels = sizes.next().captured(1).toInt();
                        QVERIFY2(scale.contains(pixels), qPrintable(QStringLiteral("%1px in %2").arg(pixels).arg(selector)));
                    }
                }
            }
        }
        TimelineWidget timeline;
        QVERIFY(scale.contains(timeline.badgeFont().pixelSize()));
    }

    // Colours drawn one on another, each with the ratio it must reach there
    // (4.5:1 for text, 3:1 for symbols, edges and handles), in both
    // appearances; a translucent one is laid on what is under it first.
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
        for (const bool dark : {false, true}) {
            const EditorTheme t = makeEditorTheme(dark);
            const LauncherTheme l = launcherTheme(dark);
            std::vector<Pair> pairs{
                {"chosen mask tool's symbol on its wash", t.accentText, t.accentTint, 3.0},
                {"chosen mask tool's symbol on its wash, held", t.accentText, t.accentTintPressed, 3.0},
                {"selected mask's name on its wash", t.buttonText, t.accentTint, 4.5},
                {"selected mask's name on its wash, held", t.buttonText, t.accentTintPressed, 4.5},
                {"playhead's head on the ruler's top", t.playhead, t.ruler, 3.0},
                {"playhead's head on the ruler's foot", t.playhead, t.rulerBottom, 3.0},
                {"playhead's line on its halo", t.playhead, t.playheadHalo, 3.0},
                {"text on a selected row", t.selectionText, t.selection, 4.5},
                {"secondary text on a selected row", t.onAccentSecondary, t.selection, 4.5},
                {"chosen menu item", t.menuSelectionText, t.menuSelection, 4.5},
                {"default button's label", t.onAccent, t.accent, 4.5},
                {"check box's tick", t.onAccent, t.accent, 3.0},
                {"track number on its header", t.trackBadgeText, t.trackHeader, 4.5},
                {"track toggle's symbol, on", t.trackToggleOn, t.trackHeader, 3.0},
                {"track toggle's symbol, on, under the pointer", t.trackToggleOn, laidOn(t.trackHeader, t.trackToggleHover), 3.0},
                {"picture clip's name", t.clipVideo.label, t.clipVideo.header, 4.5},
                {"sound clip's name", t.clipAudio.label, t.clipAudio.header, 4.5},
                {"music clip's name", t.clipMusic.label, t.clipMusic.header, 4.5},
                {"title clip's name", t.clipGenerator.label, t.clipGenerator.header, 4.5},
                {"caption clip's name", t.clipCaption.label, t.clipCaption.header, 4.5},
                {"selected clip's ring on its outer line", t.clipSelectedRing, t.clipSelectedRingOuter, 4.5},
                {"Model field under the pointer", t.text, laidOn(t.field, t.hover), 4.5},
                {"Model field under the pointer while sending", t.text, laidOn(t.assistantSurface, t.hover), 4.5},
                {"disclosure chevron on a dialog", t.textSecondary, t.dialog, 3.0},
                {"disclosure chevron on a dialog's group", t.textSecondary, laidOn(t.dialog, t.raised), 3.0},
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
                {"Source monitor's message on the stage", t.textSecondary, t.viewerSurround, 4.5},
                {"chat's secondary text", t.chatSecondary, t.chatSurface, 4.5},
                {"chat's reply expander", t.chatExpander, t.chatSurface, 4.5},
                {"Kerf's monogram on its avatar", t.chatAvatarKerfText, t.chatAvatarKerf, 4.5},
                {"the user's initials on their avatar", t.chatAvatarUserText, t.chatAvatarUser, 4.5},
                {"chip's label on its wash", t.chatChipText, laidOn(t.chatSurface, t.chatChip), 4.5},
                {"open chat toggle's symbol on its disc", t.railGlyphActive, laidOn(t.viewerSurround, t.railButtonActive), 3.0},
                {"rail symbol on its disc", t.railGlyph, laidOn(t.viewerSurround, t.railButton), 3.0},
                {"rail label on the stage", t.railLabel, t.viewerSurround, 4.5},
                {"rail symbol on its disc under the pointer", t.railGlyph, laidOn(t.viewerSurround, t.railButtonHover), 3.0},
                {"chat's text on its surface", t.chatText, t.chatSurface, 4.5},
                {"chat's error tag", t.errorText, t.chatSurface, 4.5},
                {"chat's header buttons' symbols on their wash", t.chatText, laidOn(t.chatSurface, t.chatChipHover), 3.0},
                {"a comment's action under the pointer", t.chatText, laidOn(t.chatSurface, t.chatChip), 4.5},
                {"a comment's quiet action", t.chatSecondary, t.chatSurface, 4.5},
                {"chip's label under the pointer", t.chatChipText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"chip's label on the timeline's bar", t.chatChipText, laidOn(t.chrome, t.chatChip), 4.5},
                {"Send and Accept's label on the accent", t.onAccent, t.accent, 4.5},
                {"Reject and Stop's label on their tonal fill", t.pillTonalText, laidOn(t.chatSurface, t.pillTonal), 4.5},
                {"Reject and Stop's label under the pointer", t.pillTonalText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"the composer's underline in focus", t.chatComposerRuleFocus, t.chatSurface, 3.0},
                {"the composer's text", t.chatText, t.chatSurface, 4.5},
                {"the composer's placeholder", t.placeholderText, t.chatSurface, 4.5},
                {"the model's name under the pointer", t.chatText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"launcher: Details chevron on the content", l.secondary, l.content, 3.0},
                {"launcher: scroll bar handle on the content", l.scrollHandle, l.content, 3.0},
                {"launcher: scroll bar handle on a group", l.scrollHandle, l.group, 3.0},
                {"launcher: scroll bar handle on the sidebar", l.scrollHandle, l.sidebar, 3.0},
                {"launcher: missing project on an inactive selection", l.secondaryOnSelection, l.selectionInactive, 4.5},
                {"launcher: text on a selected row", l.selectionText, l.selection, 4.5},
                {"launcher: secondary text on a selected row", l.onAccentSecondary, l.selection, 4.5},
                {"launcher: Choose's label", l.onAccent, l.accent, 4.5},
                {"launcher: Choose's label, lit", l.onAccent, l.accentTop, 4.5},
                {"launcher: sidebar symbol", l.accent, l.sidebar, 3.0},
                {"launcher: format tile's frame", l.tileGlyph, l.tile, 3.0},
                {"launcher: format tile's plus", l.tileGlyph, l.badge, 3.0},
                {"launcher: switch's knob on the accent", l.knobOn, l.accent, 3.0},
                {"launcher: tooltip", l.tooltipText, l.tooltip, 4.5},
            };
            for (const QByteArray marker : {"red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white", ""}) {
                const QColor color = t.marker(QString::fromLatin1(marker));
                const QByteArray name = marker.isEmpty() ? QByteArray("uncoloured") : marker;
                pairs.push_back({name + " marker on the ruler's top", color, t.ruler, 3.0});
                pairs.push_back({name + " marker on the ruler's foot", color, t.rulerBottom, 3.0});
            }
            for (const Pair &pair : pairs)
                QTest::addRow("%s (%s), %s", pair.what.constData(), qPrintable(pair.foreground.name(QColor::HexArgb)),
                              dark ? "dark" : "light")
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
    // (the ruler for their heads, their solid halo for their lines); and a
    // scroll bar's handle under the pointer or held reads more than at rest.
    void quietAndActiveStatesStayApart() {
        for (const bool dark : {false, true}) {
            const EditorTheme t = makeEditorTheme(dark);
            QVERIFY(t.playheadIdle.hsvSaturation() < 70);
            QCOMPARE(t.playheadHalo.alpha(), 255);
            for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
                QVERIFY2(contrastRatio(t.playheadIdle, surface) + 0.35 <= contrastRatio(t.playhead, surface),
                         qPrintable(QStringLiteral("%1 and %2 on %3").arg(t.playheadIdle.name(), t.playhead.name(), surface.name())));
            // The playhead is Kerf's accent, as it is.
            QCOMPARE(t.playhead.rgb(), kerfAccent(dark).rgb());
            const LauncherTheme l = launcherTheme(dark);
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
        QVERIFY(launcherStyleSheet(launcherTheme(true)).contains(QStringLiteral("QScrollBar::handle:vertical:pressed")));
    }

    // The chosen mask tool and the selected mask stand out from the others,
    // at rest, under the pointer and held (darkening while held, as every
    // bezel does), in the Inspector and in the Program panel's title
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
            window.m_inspector->selectMask(QStringLiteral("k1"));
            window.m_inspector->section(QStringLiteral("masks"))->setOpen(true);
            window.m_workspace->panel(QStringLiteral("inspector"))->toggleView(true);
            window.m_workspace->panel(QStringLiteral("inspector"))->setAsCurrentTab();
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // The bezel's fill, clear of its edge and its symbol or label (a
            // bare 22pt mask tool's symbol nearly fills it: its top middle).
            const auto spot = [](QToolButton *button) {
                return button->objectName() == QLatin1String("maskTool") ? QPoint(button->width() / 2, 3) : QPoint(3, button->height() / 2);
            };
            const auto fill = [&spot](QToolButton *button) {
                const QImage image = button->grab().toImage();
                return image.pixelColor(spot(button) * image.devicePixelRatio()).rgb();
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

            // The chosen tool is a quiet accent wash with no outline, its
            // symbol the accent, which reads on the wash; the others are bare
            // symbols on the panel (no bezel), a wash under the pointer.
            auto *rectangle = window.findChild<QAction *>(QStringLiteral("maskRectangle"));
            QVERIFY(rectangle && rectangle->isEnabled());
            rectangle->trigger();
            QVERIFY(rectangle->isChecked());
            QToolButton *chosen = nullptr;
            QToolButton *other = nullptr;
            for (QToolButton *tool : window.m_inspector->findChildren<QToolButton *>(QStringLiteral("maskTool")))
                (tool->defaultAction() == rectangle ? chosen : other) = tool;
            QVERIFY(chosen && other && chosen->isVisible() && chosen->isChecked() && !other->isChecked());
            // (A bare button grabs clear where it paints nothing.)
            const auto clear = [&spot](QToolButton *button) {
                const QImage image = button->grab().toImage();
                return image.pixelColor(spot(button) * image.devicePixelRatio()).alpha() == 0;
            };
            QVERIFY(clear(other));
            QVERIFY(hovered(other) != fill(other) && held(other) != fill(other) && held(other) != hovered(other));
            for (const QRgb wash : {fill(chosen), hovered(chosen)}) QCOMPARE(wash, theme.accentTint.rgb());
            QCOMPARE(held(chosen), theme.accentTintPressed.rgb());
            // Its edge is the wash (or the panel behind it), never a line in the accent.
            const QImage chosenShot = chosen->grab().toImage();
            for (const int x : {0, 1})
                QVERIFY(contrastRatio(chosenShot.pixelColor(QPoint(x, chosen->height() / 2) * chosenShot.devicePixelRatio()),
                                      theme.accentText) >= 2.0);
            QVERIFY(qGray(theme.accentTintPressed.rgb()) < qGray(theme.accentTint.rgb()));
            QCOMPARE(ink(chosen->icon(), QIcon::Normal, QIcon::On), theme.accentText.rgb());
            // At rest the others are quiet (the secondary ink, as the headers
        // they sit in); full ink under the pointer.
        QCOMPARE(ink(other->icon()), theme.textSecondary.rgb());
        QCOMPARE(ink(other->icon(), QIcon::Active, QIcon::Off), theme.text.rgb());
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
            QVERIFY(clear(chosen));

            // So is the selected mask's name; the other's is plain words, no wash.
            QToolButton *selected = nullptr;
            QToolButton *unselected = nullptr;
            for (QToolButton *name : window.m_inspector->findChildren<QToolButton *>(QStringLiteral("maskName")))
                (name->text() == QStringLiteral("k1") ? selected : unselected) = name;
            QVERIFY(selected && unselected && selected->isVisible() && selected->isChecked() && !unselected->isChecked());
            for (const QRgb wash : {fill(selected), hovered(selected)}) QCOMPARE(wash, theme.accentTint.rgb());
            QCOMPARE(held(selected), theme.accentTintPressed.rgb());
            QVERIFY(fill(unselected) != theme.accentTint.rgb());

            // The Model field's text is drawn in one colour, hovered or not,
            // so that colour reads on the hover wash.
            QLineEdit *model = window.m_modelSelect->lineEdit();
            QVERIFY(model);
            model->ensurePolished();
            const QColor modelText = model->palette().color(QPalette::Active, QPalette::Text);
            QCOMPARE(modelText.rgb(), theme.chatText.rgb());
            QVERIFY(contrastRatio(modelText, laidOn(theme.chatSurface, theme.chatChipHover)) >= 4.5);
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
            const QColor bezel = makeEditorTheme(dark).button;
            InspectorPanel::paintWell(&well, value.name());
            const QImage swatch = well.icon().pixmap(well.iconSize(), 2.0).toImage();
            QCOMPARE(swatch.size(), well.iconSize() * 2);
            QCOMPARE(swatch.pixelColor(swatch.width() / 2, swatch.height() / 2).rgb(), value.rgb());
            // The ring's outer half, solid at twice the size, as it shows on the bezel.
            const QColor ring = laidOn(bezel, swatch.pixelColor(0, swatch.height() / 2));
            QVERIFY2(contrastRatio(ring, value) >= 3.0, qPrintable(ring.name()));
            // It shows on the light bezel, and on the dark one keeps to the
            // bezel's own grey rather than ringing the swatch in light.
            // (The dark bezel is a deep espresso, so a ring that reads on
            // black stays a step above it, never a light halo.)
            if (!dark) QVERIFY2(contrastRatio(ring, bezel) >= 3.0, qPrintable(ring.name()));
            else QVERIFY2(contrastRatio(ring, bezel) < 2.0 && ring.lightness() < bezel.lightness() + 50, qPrintable(ring.name()));
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
            for (const QColor &surface : {theme.window, theme.panel, theme.chrome, theme.well, theme.field, theme.dialog,
                                          theme.chatSurface, theme.viewerSurround})
                QVERIFY2(isWarmPaper(surface, dark), qPrintable(surface.name()));
            QVERIFY(dark ? theme.text.lightness() > 180 : theme.text.lightness() < 60);
            // Text is warm near-black or cream, never pure black or white.
            QVERIFY(theme.text.rgb() != QColor(Qt::black).rgb() && theme.text.rgb() != QColor(Qt::white).rgb());
            QVERIFY(theme.textStrong.rgb() != QColor(Qt::black).rgb() && theme.textStrong.rgb() != QColor(Qt::white).rgb());
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
            // A track's number reads on its header as text; its toggles' bare
            // symbols, when on, as glyphs, on the hover wash too.
            QVERIFY(contrastRatio(theme.trackBadgeText, theme.trackHeader) >= 4.5);
            QVERIFY(contrastRatio(theme.trackToggleOn, laidOn(theme.trackHeader, theme.trackToggleHover)) >= 3.0);
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
            // The empty viewer's hint is on the black card; the Source
            // monitor's message is on the stage, which follows the appearance.
            QVERIFY(contrastRatio(theme.viewerTextDim, theme.viewerCanvas) >= 4.5);
            QVERIFY(contrastRatio(theme.viewerText, theme.viewerCanvas) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.viewerSurround) >= 4.5);
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
            // The viewer's stage follows the appearance (cream or espresso,
            // a step deeper than the panels), and the picture card is the
            // same warm black in both.
            QCOMPARE(theme.viewerSurround.rgb(), (dark ? QColor("#120e0b") : QColor("#e6dece")).rgb());
            QCOMPARE(theme.viewerCanvas.rgb(), QColor("#0a0807").rgb());

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
            QVERIFY(window.chatOpen());
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
            // a group of its own, is chosen: a quiet accent wash with no
            // outline, its bubble the accent, as a chosen mode in a title strip.
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
            // read on: the Source monitor's, a tab behind the Program
            // monitor (whose time is on its picture, in the player's white).
            QList<QLabel *> times = window.m_sourceMonitor->findChildren<QLabel *>(QStringLiteral("transportTime"));
            QCOMPARE(times.size(), 1);
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
            // A tab row's details are plain text on the panel's own surface
            // (text tabs have no strip), not a dead button.
            {
                auto *detail = window.findChild<QToolButton *>(QStringLiteral("viewerDetail"));
                QVERIFY(detail && detail->isVisible() && !detail->isEnabled());
                QCOMPARE(dominantColor(detail->parentWidget()->grab(detail->geometry()).toImage()), theme.panel.rgb());
                auto *tracks = window.findChild<QLabel *>(QStringLiteral("timelineDetail"));
                QVERIFY(tracks && tracks->isVisible());
                QCOMPARE(dominantColor(tracks->parentWidget()->grab(tracks->geometry()).toImage()), theme.panel.rgb());
                QCOMPARE(tracks->palette().color(QPalette::WindowText).rgb(), theme.textSecondary.rgb());
            }
            save(image, QStringLiteral("editor-%1").arg(name));
            // With the conversation closed, the timeline's bar stands in for
            // it: the window's chrome, no coloured stripe at its start, not
            // amber, and 22px tonal chips for its buttons (no outlined box).
            window.setChatOpen(false);
            QCoreApplication::processEvents();
            QVERIFY(window.m_aiBar->isVisible());
            {
                const QImage aiBar = window.m_aiBar->grab().toImage();
                QCOMPARE(dominantColor(aiBar), theme.chrome.rgb());
                QCOMPARE(aiBar.pixelColor(QPoint(1, aiBar.height() / 2) * aiBar.devicePixelRatio()).rgb(), theme.chrome.rgb());
                QCOMPARE(amberPixels(aiBar), 0);
                const QImage shown = window.grab().toImage();
                for (QToolButton *control : window.m_aiBar->findChildren<QToolButton *>()) {
                    if (!control->isVisible()) continue;
                    QCOMPARE(control->height(), 22);
                    if (control->objectName() == QStringLiteral("aiAccept")) continue;
                    const QImage chip = shown.copy(QRect(control->mapTo(&window, QPoint()) * shown.devicePixelRatio(),
                                                         control->size() * shown.devicePixelRatio()));
                    QVERIFY2(nearColor(dominantColor(chip), laidOn(theme.chrome, theme.chatChip).rgb()), qPrintable(control->text()));
                }
            }
            window.setChatOpen(true);
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
            // The proposal's default button, Accept, is a pill filled with the accent.
            QPushButton *primary = window.m_acceptButton;
            QVERIFY(primary->isVisible() && primary->isEnabled());
            QCOMPARE(primary->objectName(), QStringLiteral("chatPillFilled"));
            // With the proposal scrolled away under the latest comment, the
            // compact bar's Accept stands in for it, filled the same.
            QTRY_VERIFY(window.m_chat->proposalBar()->isVisible() || window.m_chatScroll->verticalScrollBar()->value() == 0);
            QPushButton *shown = window.m_chat->proposalBar()->isVisible() ? window.m_chat->barAcceptButton() : primary;
            QCOMPARE(shown->objectName(), QStringLiteral("chatPillFilled"));
            const QImage states = window.grab().toImage();
            QCOMPARE(states.pixelColor(shown->mapTo(&window, QPoint(shown->height() / 2 - 6, shown->height() / 2)) * states.devicePixelRatio()).rgb(),
                     theme.accent.rgb());
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
            QCOMPARE(window.m_acceptButton->height(), ChatPanel::kPillHeight);
            // So does every other row of buttons, as macOS orders them: the
            // timeline's bar (shown with the conversation closed) ends in
            // Accept and the banner in Load from Disk, each 8px past the
            // button before it.
            window.setChatOpen(false);
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
            // Kerf's suggestions are tonal chips sized to their labels, with
            // no outline: on the timeline's bar at its controls' height...
            {
                const auto chips = window.m_nextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep"));
                QVERIFY(!chips.isEmpty());
                const QImage bar = window.grab().toImage();
                for (QPushButton *chip : chips) {
                    QCOMPARE(chip->height(), 22);
                    const QRect area(chip->mapTo(&window, QPoint()) * bar.devicePixelRatio(), chip->size() * bar.devicePixelRatio());
                    QVERIFY(nearColor(dominantColor(bar.copy(area)), laidOn(theme.chrome, theme.chatChip).rgb()));
                    QVERIFY(chip->width() < 160);
                }
            }
            window.setChatOpen(true);
            QCoreApplication::processEvents();
            // ...and over the composer, a chip's 28px.
            {
                const auto chips = window.m_chatNextSteps->findChildren<QPushButton *>(QStringLiteral("nextStep"));
                QVERIFY(!chips.isEmpty());
                const QImage panel = window.grab().toImage();
                for (QPushButton *chip : chips) {
                    QCOMPARE(chip->height(), ChatPanel::kChipHeight);
                    const QRect area(chip->mapTo(&window, QPoint()) * panel.devicePixelRatio(), chip->size() * panel.devicePixelRatio());
                    QVERIFY(nearColor(dominantColor(panel.copy(area)), laidOn(theme.chatSurface, theme.chatChip).rgb()));
                    QVERIFY(chip->width() < 180);
                }
            }
            // The bar and its text are neutral; only the 14-point triangle is orange.
            for (QWidget *part : window.m_divergedBanner->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
                if (part != window.m_divergedGlyph) QCOMPARE(amberPixels(part->grab().toImage()), 0);
            QCOMPARE(dominantColor(window.m_divergedBanner->grab().toImage()), theme.warningSurface.rgb());
            // The banner is its own bar, not a second row of the toolbar: in
            // dark a step lighter than it, in light a warm tint.
            QVERIFY(theme.warningSurface.rgb() != theme.chrome.rgb());
            if (dark) QVERIFY(theme.warningSurface.lightness() > theme.chrome.lightness());
            // The conversation's view, scrolled away from its top, has a
            // hairline under its header (or the compact proposal bar), as a
            // comments panel's header does.
            QVERIFY(window.m_chatScroll->verticalScrollBar()->value() > 0);
            auto *scrollEdge = window.findChild<QFrame *>(QStringLiteral("scrollEdge"));
            QVERIFY(scrollEdge && scrollEdge->isVisible());
            {
                const QImage shown = window.grab().toImage();
                QCOMPARE(shown.pixelColor(scrollEdge->mapTo(&window, QPoint(4, 0)) * shown.devicePixelRatio()).rgb(), theme.divider.rgb());
            }
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

            // The Inspector: every row's controls start after one 84pt label
            // column, rows are 32pt apart, a colour is a well with its hex, a
            // file is a field with a folder in it (a look, as FilmCraft's LUT
            // parameters, stands in for one), and each effect is a header
            // with its switch, name and ⋯, a hairline above each section.
            window.m_inspector->m_effectDefs.insert(QStringLiteral("test.look"), QJsonObject{
                {"id", "test.look"}, {"name", "Look"}, {"params", QJsonArray{QJsonObject{
                    {"name", "look_lut"}, {"type", "file"}, {"doc", "Look LUT"}, {"extensions", QJsonArray{"cube", "3dl"}}, {"default", ""}}}}});
            window.m_inspector->m_shape.clear();
            QSettings().setValue(QStringLiteral("editor/inspector/fx/light.edgeglow/open"), true);
            QSettings().setValue(QStringLiteral("editor/inspector/fx/test.look/open"), true);
            const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
            window.applySnapshot(QJsonObject{{"revision", 14}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24},
                {"sources", QJsonArray{QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "wide"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"effects", QJsonArray{QJsonObject{{"fx", "light.edgeglow"}, {"enabled", false}, {"params", QJsonObject{}}},
                                                       QJsonObject{{"fx", "test.look"}, {"enabled", true},
                                                                   {"params", QJsonObject{{"look_lut", "looks/harbor-dusk.cube"}}}}}}}}}}}}}}});
            window.selectClip(QStringLiteral("c1"));
            window.m_workspace->panel(QStringLiteral("inspector"))->toggleView(true);
            window.m_workspace->panel(QStringLiteral("inspector"))->setAsCurrentTab();
            InspectorPanel *inspector = window.m_inspector;
            inspector->section(QStringLiteral("effects"))->setOpen(true);
            QCoreApplication::processEvents();
            QVERIFY(inspector->isVisible());
            const auto rows = inspector->findChildren<InspectorRow *>();
            QVERIFY(rows.size() > 6);
            for (InspectorRow *row : rows) {
                if (!row->isVisibleTo(inspector)) continue;
                QCOMPARE(row->label()->width(), InspectorRow::kLabelWidth);
                QCOMPARE(row->height(), InspectorRow::kHeight);
                int left = row->width();
                for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly))
                    if (child != row->label() && child->isVisibleTo(row) && child->width() > InspectorRow::kResetLane) left = std::min(left, child->x());
                QCOMPARE(left, InspectorRow::kLabelWidth + 8);
            }
            // The file's row: its field, the folder inside it.
            auto *lookFile = inspector->findChild<QWidget *>(QStringLiteral("effectParam_look_lut"));
            QVERIFY(lookFile && lookFile->findChild<QAction *>(QStringLiteral("chooseFile")));
            // The file's name, its whole path in the tooltip; the name left as
            // shown keeps the path, a typed path replaces it.
            QLineEdit *lookLine = lookFile->findChild<QLineEdit *>();
            QCOMPARE(lookLine->text(), QStringLiteral("harbor-dusk.cube"));
            QCOMPARE(lookLine->toolTip(), QStringLiteral("looks/harbor-dusk.cube"));
            window.m_lastCommand = {};
            emit lookLine->editingFinished();
            QVERIFY(takeOps(window).isEmpty());
            lookLine->setText(QStringLiteral("looks/harbor-night.cube"));
            emit lookLine->editingFinished();
            QCOMPARE(takeOps(window).first().toObject().value("params").toObject().value("look_lut").toString(), QStringLiteral("looks/harbor-night.cube"));
            // (No snapshot answers the edit here: the field shows its value again.)
            window.m_inspector->refreshValues();
            QCOMPARE(lookLine->text(), QStringLiteral("harbor-dusk.cube"));
            // A colour is a well, its hex beside it.
            auto *well = inspector->findChild<QPushButton *>(QStringLiteral("effectParam_color"));
            QVERIFY(well);
            QCOMPARE(well->size(), QSize(44, 22));
            QVERIFY(well->text().isEmpty());
            QCOMPARE(qobject_cast<QLabel *>(well->parentWidget()->findChild<QLabel *>(QStringLiteral("colorHex")))->text(), QStringLiteral("#4DCCFF"));
            // A switch, not a check box, for an effect's bool and for each effect.
            QVERIFY(inspector->findChildren<QCheckBox *>().isEmpty());
            QCOMPARE(inspector->findChildren<InspectorSwitch *>(QStringLiteral("effectEnabled")).size(), 2);
            for (QLabel *title : inspector->findChildren<QLabel *>(QStringLiteral("effectName"))) QCOMPARE(title->font().weight(), QFont::DemiBold);
            // The picture shows the whole Inspector: the window grows until
            // it needs no scrolling.
            QScrollBar *inspectorScroll = inspector->scrollArea()->verticalScrollBar();
            for (int grow = 0; inspectorScroll->maximum() > 0 && grow < 20; ++grow) {
                window.resize(window.width(), window.height() + 20);
                QCoreApplication::processEvents();
            }
            const QImage inspectorImage = window.m_workspace->panel(QStringLiteral("inspector"))->grab().toImage();
            // The hairline over the Effects section runs across the column, on the panel.
            {
                const QWidget *header = inspector->section(QStringLiteral("effects"))->header();
                const QPoint rule = header->mapTo(window.m_workspace->panel(QStringLiteral("inspector")), QPoint(header->width() / 2, 0));
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
        // docks, and the conversation's panel and the comments it shows.
        setEditorColorScheme(ColorScheme::Light);
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("Done."));
        QCoreApplication::processEvents();
        const auto surface = [&window] {
            const QImage panel = window.m_chat->grab().toImage();
            return panel.pixelColor(QPoint(4, window.m_chat->height() / 2) * panel.devicePixelRatio()).rgb();
        };
        const auto nameInk = [&window] {
            QLabel *name = window.m_chat->rows().constFirst()->nameLabel();
            name->ensurePolished();
            return name->palette().color(QPalette::WindowText).rgb();
        };
        QCOMPARE(surface(), editorTheme().chatSurface.rgb());
        QCOMPARE(nameInk(), editorTheme().chatText.rgb());
        const int before = changed.count();
        setEditorColorScheme(ColorScheme::Dark);
        QCOMPARE(changed.count(), before + 1);
        QVERIFY(window.m_workspace->manager()->styleSheet().contains(editorTheme().panel.name()));
        QCoreApplication::processEvents();
        QCOMPARE(surface(), editorTheme().chatSurface.rgb());
        QCOMPARE(nameInk(), editorTheme().chatText.rgb());
        QVERIFY(!window.m_disclosures.isEmpty());
        for (QToolButton *disclosure : std::as_const(window.m_disclosures))
            QCOMPARE(ink(disclosure->icon()), editorTheme().textSecondary.rgb());
        // Re-applying the same appearance changes nothing.
        applyEditorAppearance();
        QCOMPARE(changed.count(), before + 1);
    }

    // The structure the owner chose (SPEC-R10 part D): a title with a quiet
    // subtitle of the project's state; text tabs with an underline; a 300
    // column holding "Search media" over a two-column grid of tiles; a stage
    // row over a 290 timeline that shows eight tracks at 1440x900 and six at
    // 1280x800, its header saying how many tracks and how long, with a zoom;
    // and the selection on the status bar's left, ⌘-click adding to it.
    void editorTakesTheChosenStructure() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto clip = [](const char *id, const char *source, double start, double duration) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", source}, {"timelineStart", start}, {"duration", duration}};
        };
        QJsonArray tracks;
        for (int index = 1; index <= 4; ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("V%1").arg(index)}, {"type", "video"},
                                      {"clips", index == 1 ? QJsonArray{clip("v1a", "wide", 0.0, 6.0), clip("v1b", "close", 6.0, 5.0)} : QJsonArray{}}});
        tracks.append(QJsonObject{{"id", "S1"}, {"type", "subtitle"}, {"name", "Captions"}, {"clips", QJsonArray{}}});
        for (int index = 1; index <= 4; ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("A%1").arg(index)}, {"type", "audio"}, {"clips", QJsonArray{}}});
        const QJsonArray sources{
            QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}},
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"width", 1080}, {"height", 1920}, {"duration", 20}},
            QJsonObject{{"id", "drone"}, {"path", "media/drone-pass.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}}};
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 18}, {"timeline", QJsonObject{
            {"name", "Harbor Teaser"}, {"fps", 24}, {"width", 1920}, {"height", 1080}, {"sources", sources}, {"tracks", tracks}}}});
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        fakeArtwork(window.m_artwork);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        QCoreApplication::processEvents();
        const EditorTheme &theme = editorTheme();

        // The title, and under it the project's state, centred together.
        QCOMPARE(window.m_projectTitle->text(), QStringLiteral("Harbor Teaser"));
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Saved · 16:9 · 24 fps"));
        QVERIFY(window.m_projectSubtitle->geometry().top() >= window.m_projectTitle->geometry().bottom());
        QCOMPARE(window.m_projectSubtitle->geometry().center().x(), window.m_projectTitle->geometry().center().x());
        QJsonObject edited = sharedStatus();
        edited.insert(QStringLiteral("changed"), true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", edited}});
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · 16:9 · 24 fps"));
        // The toolbar's format pop-up names the format as the subtitle and
        // the Inspector's Frame size do: "1:1 · 1080 × 1080", one separator,
        // never "Custom" for a preset, never two spaces in a row.
        {
            const QString times = QStringLiteral("\u2009×\u2009");
            QCOMPARE(window.m_canvasPreset->currentText(), QStringLiteral("16:9 · 1920") + times + QStringLiteral("1080"));
            const struct {
                QSize size;
                QString ratio;
                QString text;
            } formats[] = {{QSize(1080, 1080), QStringLiteral("1:1"), QStringLiteral("1:1 · 1080") + times + QStringLiteral("1080")},
                           {QSize(1080, 1350), QStringLiteral("4:5"), QStringLiteral("4:5 · 1080") + times + QStringLiteral("1350")},
                           {QSize(1080, 1920), QStringLiteral("9:16"), QStringLiteral("9:16 · 1080") + times + QStringLiteral("1920")},
                           {QSize(1280, 720), QStringLiteral("16:9"), QStringLiteral("Custom · 1280") + times + QStringLiteral("720")}};
            for (const auto &format : formats) {
                window.syncCanvasPreset(format.size);
                QCOMPARE(window.m_canvasPreset->currentText(), format.text);
                QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · %1 · 24 fps").arg(format.ratio));
                QCOMPARE(canvasSizeLabel(format.size), format.text);
                for (int index = 0; index < window.m_canvasPreset->count(); ++index)
                    QVERIFY2(!window.m_canvasPreset->itemText(index).contains(QStringLiteral("  ")), qPrintable(window.m_canvasPreset->itemText(index)));
            }
            window.syncCanvasPreset(QSize(1920, 1080));
            QCOMPARE(window.m_canvasPreset->currentText(), QStringLiteral("16:9 · 1920") + times + QStringLiteral("1080"));
        }

        // Text tabs: 13pt names on the panel's own surface, the chosen one in
        // the strongest ink over a 2pt underline of it, the others quieter,
        // a hairline under the row; no strip, no boxes.
        for (const char *id : {"project", "inspector", "effects", "presets", "program", "source", "timeline"}) {
            ads::CDockWidget *dock = window.m_workspace->panel(QString::fromLatin1(id));
            ads::CDockWidgetTab *tab = dock->tabWidget();
            QVERIFY2(tab->property("textTab").toBool(), id);
            auto *name = tab->findChild<QLabel *>();
            QVERIFY(name);
            QCOMPARE(name->font().pixelSize(), 13);
            const bool chosen = dock->dockAreaWidget()->currentDockWidget() == dock;
            // (A tab under the pointer takes the body's ink.)
            if (tab->underMouse()) continue;
            QVERIFY2(name->palette().color(QPalette::WindowText).rgb() == (chosen ? theme.textStrong : theme.tabInactiveText).rgb(),
                     qPrintable(QStringLiteral("%1 %2 %3").arg(QString::fromLatin1(id)).arg(chosen).arg(name->palette().color(QPalette::WindowText).name())));
            QCOMPARE(tab->palette().color(QPalette::Window).alpha() == 0 || !tab->autoFillBackground(), true);
            QCOMPARE(dock->dockAreaWidget()->titleBar()->height(), 32);
        }
        QVERIFY(window.m_workspace->manager()->styleSheet().contains(QStringLiteral("border-bottom: 2px solid ") + theme.textStrong.name()));
        // The Program tab row ends with the frame, the rate and the fit.
        QCOMPARE(window.m_viewerDetail->text(), QStringLiteral("1920 × 1080  ·  24 fps  ·  Fit"));

        // The column: about 300 at 1440x900, "Search media" over two columns
        // of 124 by 70 tiles, 16 in from its edge.
        ads::CDockAreaWidget *column = window.m_workspace->panel(QStringLiteral("project"))->dockAreaWidget();
        QVERIFY2(std::abs(column->width() - 300) <= 6, qPrintable(QString::number(column->width())));
        ProjectPanel *bin = window.m_projectPanel;
        QCOMPARE(bin->search()->placeholderText(), QStringLiteral("Search media"));
        QCOMPARE(bin->m_list->count(), 4);
        const QRect first = bin->m_list->visualItemRect(bin->m_list->item(0));
        const QRect second = bin->m_list->visualItemRect(bin->m_list->item(1));
        QCOMPARE(second.top(), first.top());
        QVERIFY(second.left() > first.right() - 1);
        const int tileLeft = bin->m_list->viewport()->mapTo(bin, first.topLeft()).x() + (first.width() - ProjectPanel::kThumbnail.width()) / 2;
        QCOMPARE(tileLeft, ProjectPanel::kSidePadding);
        QCOMPARE(bin->m_list->item(0)->data(Qt::UserRole + 1).toString(), QStringLiteral("16:9"));
        QCOMPARE(bin->m_list->item(1)->data(Qt::UserRole + 1).toString(), QStringLiteral("9:16"));
        QCOMPARE(bin->m_list->item(3)->data(Qt::UserRole + 1).toString(), QStringLiteral("Audio"));
        // Search filters the bin by name.
        bin->search()->setText(QStringLiteral("DRONE"));
        QStringList shown;
        for (int row = 0; row < bin->m_list->count(); ++row)
            if (!bin->m_list->item(row)->isHidden()) shown << bin->m_list->item(row)->data(Qt::UserRole).toString();
        QCOMPARE(shown, QStringList{QStringLiteral("drone")});
        bin->search()->clear();
        QVERIFY(!bin->m_list->item(0)->isHidden());

        // The timeline's header: after its name, how many tracks and how
        // long; on the right, the zoom beside the track heights.
        QCOMPARE(window.m_timelineDetail->text(), QStringLiteral("9 tracks  ·  00:00:18:00"));
        QVERIFY(window.m_timelineDetail->isVisible() && window.m_timelineZoom->isVisible());
        ads::CDockAreaTitleBar *header = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->titleBar();
        QCOMPARE(window.m_timelineDetail->parentWidget(), header);
        QCOMPARE(header->indexOf(window.m_timelineDetail), header->indexOf(header->tabBar()) + 1);
        QVERIFY(window.m_timelineZoom->x() > header->width() / 2);
        auto *heights = header->findChild<QToolButton *>(QStringLiteral("trackHeightMini"));
        QVERIFY(heights && window.m_timelineZoom->geometry().right() < heights->x());
        // Eight tracks at their default heights fit at 1440x900.
        const auto tracksShown = [&window] {
            QCoreApplication::processEvents();
            const int bottom = window.m_timelineScroll->viewport()->height() - window.m_timeline->pos().y();
            int fit = 0;
            for (int row = 0; row < window.m_timeline->trackRowCount(); ++row)
                if (window.m_timeline->trackHeaderRect(row).bottom() < bottom) ++fit;
            return fit;
        };
        QVERIFY2(tracksShown() >= 8, qPrintable(QString::number(tracksShown())));
        // The zoom sets the timeline's scale, and follows it.
        const double before = window.m_timeline->scale();
        window.m_timelineZoom->zoomInButton()->click();
        QVERIFY(window.m_timeline->scale() > before * 1.2);
        window.m_timelineZoom->slider()->setValue(0);
        QVERIFY(std::abs(window.m_timeline->scale() - TimelineWidget::kMinScale) < 0.5);
        window.m_timeline->setScale(120.0);
        QCOMPARE(window.m_timelineZoom->slider()->value(), TimelineZoomControl::position(120.0));

        // The selection on the status bar's left: a click, then ⌘-click adds.
        QVERIFY(window.m_selectionStatus->text().isEmpty());
        window.clickSelectClip(QStringLiteral("v1a"));
        QCOMPARE(window.m_selectionStatus->text(), QStringLiteral("1 clip selected  ·  00:00:06:00"));
        window.m_timeline->fitToWidth(window.m_timelineScroll->viewport()->width());
        QCoreApplication::processEvents();
        const QRect other = window.m_timeline->clipBounds(QStringLiteral("v1b"));
        QVERIFY(!other.isEmpty());
        QTest::mouseClick(window.m_timeline, Qt::LeftButton, Qt::ControlModifier, other.center());
        QCOMPARE(window.m_selectedClips, (QStringList{QStringLiteral("v1a"), QStringLiteral("v1b")}));
        QCOMPARE(window.m_timeline->selectedClips(), window.m_selectedClips);
        QCOMPARE(window.m_selectionStatus->text(), QStringLiteral("2 clips selected  ·  00:00:11:00"));
        QCOMPARE(window.m_inspector->selection(), window.m_selectedClips);
        // ⌘-click again takes it out; a plain click selects one.
        QTest::mouseClick(window.m_timeline, Qt::LeftButton, Qt::ControlModifier, other.center());
        QCOMPARE(window.m_selectedClips, QStringList{QStringLiteral("v1a")});
        window.clickSelectClip(QString());
        QVERIFY(window.m_selectionStatus->text().isEmpty());

        // At 1280x800 six tracks still fit.
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        QTest::qWait(20);
        QVERIFY2(tracksShown() >= 6, qPrintable(QString::number(tracksShown())));
    }

    // The whole editor in the Editing workspace with Kerf's conversation
    // open in the stage, as a Short shows with its comments open: a 16:9, a
    // 9:16 and a 1:1 project, in both appearances. The card keeps the
    // project's shape and is as large as the group allows; card, rail and
    // conversation are one group centred in the stage, the conversation as
    // tall as the card and level with it. With KERF_SCREENSHOT_DIR set, each
    // is saved as editor-<format>-<scheme>.png, the 9:16 and 16:9 ones with
    // the conversation closed as editor-<format>-closed-<scheme>.png, and the
    // 16:9 one with the laptop off as editor-16x9-off-<scheme>.png.
    void editorShowsEveryFormatBesideItsConversation() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        // A harbor at dawn in the project's shape (the tests' procedural
        // footage): sky over the water, whose lower part is a dark blue-grey.
        const auto picture = [](const QSize &size) { return testing::footageFrame(testing::Shot::HarborWide, size); };
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
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b by 36 frames"));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Removed v1c"));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Now caption the dialogue."));
                window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                                QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music under the dialogue"}},
                                                QJsonObject{{"label", "Match the colour"}, {"request", "Match the close-up to the wide"}}});
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                // Let the conversation settle at its latest comment.
                QTest::qWait(30);
                const QString label = QString::fromLatin1(format.name);
                QCOMPARE(window.m_canvasSize, format.canvas);
                verifyListEnd(window.m_chat);
                QVERIFY(window.m_viewer->isVisible() && window.m_chat->isVisible() && window.m_rail->isVisible());
                QVERIFY(!window.m_sourceMonitor->isVisible());

                // The card keeps the project's shape; card (or the phone a
                // portrait one shows in), rail and conversation are one group
                // centred in the stage.
                const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
                QVERIFY2(!stage.overlay, qPrintable(label));
                const bool tall = format.canvas.width() < format.canvas.height();
                const bool wide = format.canvas.width() > format.canvas.height();
                // A portrait picture in a phone, a wide one in a laptop, a
                // square one on the plain card (in a phone it would be under
                // half the card's width here).
                QCOMPARE(stage.kind, tall ? ViewerWidget::Device::Phone : wide ? ViewerWidget::Device::Laptop : ViewerWidget::Device::None);
                if (!tall && !wide) {
                    // Measured, as R10 asks before putting 1:1 in the phone:
                    // the square picture on the card against the same picture
                    // across a phone's screen, in this stage.
                    const ViewerWidget::StageLayout inPhone = window.m_viewer->stageLayout(ViewerWidget::Device::Phone);
                    qInfo("1:1 at 1440x900: card picture %.0f px wide, in the phone %.0f px (%.0f%%)", stage.card.width(),
                          inPhone.card.width(), 100.0 * inPhone.card.width() / stage.card.width());
                    QVERIFY(inPhone.screen.adjusted(-0.5, -0.5, 0.5, 0.5).contains(inPhone.card));
                    QVERIFY2(inPhone.card.width() < 0.5 * stage.card.width(), "a square picture loses over half its width in the phone");
                }
                const QRectF video = stage.card;
                const QRectF body = stage.body;
                QCOMPARE(window.m_viewer->videoRect(), video);
                QVERIFY2(std::abs(video.width() / video.height() - double(format.canvas.width()) / format.canvas.height()) < 0.01,
                         qPrintable(label));
                const int inset = window.m_viewer->stageInset(stage.kind);
                if (!tall) QCOMPARE(inset, ViewerWidget::kStageMargin);
                const QRectF room = QRectF(window.m_viewer->rect()).adjusted(ViewerWidget::kStageMargin, inset,
                                                                             -ViewerWidget::kStageMargin, -inset);
                const QRect panel = window.m_chat->geometry();
                QCOMPARE(panel, stage.panel);
                QCOMPARE(window.m_rail->geometry(), stage.rail);
                const int leftSpace = qRound(body.left());
                const int rightSpace = window.m_viewer->width() - panel.right() - 1;
                QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1: %2 | %3").arg(label).arg(leftSpace).arg(rightSpace)));
                QVERIFY2(room.contains(QRectF(body.left(), panel.top(), panel.right() - body.left(), panel.height()).adjusted(0.5, 0.5, -0.5, -0.5)),
                         qPrintable(label));
                // Level with the card (or the device's body) and as tall,
                // in every format, and never shorter than its least.
                QCOMPARE(panel.top(), qRound(body.top()));
                QVERIFY2(std::abs(panel.height() - body.height()) <= 1,
                         qPrintable(QStringLiteral("%1: %2 for %3").arg(label).arg(panel.height()).arg(body.height())));
                QVERIFY2(panel.height() >= ViewerWidget::kMinChatHeight, qPrintable(label));
                // A tall or square card fills the stage's height; a wide one
                // the height, or the width the group leaves it.
                if (!wide)
                    QVERIFY2(std::abs(body.height() - room.height()) < 1, qPrintable(label));
                else
                    QVERIFY2(std::abs(body.height() - room.height()) < 1 ||
                                 std::abs(body.width() - (room.width() - 2 * window.m_viewer->stageGap() - ViewerWidget::kRailWidth - panel.width())) < 1,
                             qPrintable(label));
                // Its own width beside a tall or square picture; beside a wide
                // one it narrows so the picture keeps its share.
                if (!wide)
                    QCOMPARE(panel.width(), ViewerWidget::kDefaultChatWidth);
                else
                    QVERIFY(panel.width() >= ViewerWidget::kMinChatWidth && panel.width() <= ViewerWidget::kDefaultChatWidth);
                // At this window the picture stays the larger part of the group.
                QVERIFY2(body.width() * body.height() > panel.width() * panel.height() * 0.5,
                         qPrintable(QStringLiteral("%1: body %2x%3, panel %4x%5, viewer %6x%7").arg(label).arg(body.width()).arg(body.height())
                                        .arg(panel.width()).arg(panel.height()).arg(window.m_viewer->width()).arg(window.m_viewer->height())));
                // One stage row for every shape (575 over a 290 timeline):
                // the timeline does not jump when the format changes.
                const int timelineHeight = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->height();
                QVERIFY2(timelineHeight >= 280 && timelineHeight <= 300, qPrintable(QStringLiteral("%1: timeline %2").arg(label).arg(timelineHeight)));
                if (tall) {
                    // The phone: its screen 9:18.5 inside an even bezel, the
                    // picture across its width, at least 240 points wide.
                    // A 9:16 picture in a taller screen is bounded by the
                    // stage's height, so it keeps over four fifths of the
                    // plain card's width (16/18.5 of it, less the bezel).
                    QVERIFY(body.contains(stage.screen));
                    QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kDeviceScreenAspect) < 0.01);
                    QVERIFY(std::abs((stage.screen.left() - body.left()) - kDeviceBezel * body.width()) < 0.5);
                    QVERIFY(std::abs(video.width() - stage.screen.width()) < 0.5);
                    QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(video));
                    const double plain = window.m_viewer->stageLayout(ViewerWidget::Device::None).card.width();
                    QVERIFY2(stage.screen.width() >= 240 && stage.screen.width() >= 0.82 * plain,
                             qPrintable(QStringLiteral("%1: screen %2 of %3").arg(label).arg(stage.screen.width()).arg(plain)));
                }
                if (wide) {
                    // The laptop: the 16:9 picture across its 16:10 screen,
                    // at least 90% as wide as the plain card here.
                    const auto rectText = [](const QRectF &r) { return QStringLiteral("%1,%2 %3x%4").arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height()); };
                    QVERIFY2(body.contains(stage.screen) && stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(video),
                             qPrintable(QStringLiteral("body %1 screen %2 video %3").arg(rectText(body), rectText(stage.screen), rectText(video))));
                    QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kLaptopScreenAspect) < 0.01);
                    QVERIFY(std::abs(video.width() - stage.screen.width()) < 0.5);
                    const double plain = window.m_viewer->stageLayout(ViewerWidget::Device::None).card.width();
                    QVERIFY2(video.width() >= 0.9 * plain, qPrintable(QStringLiteral("%1 of %2").arg(video.width()).arg(plain)));
                    QVERIFY(stage.screen.contains(window.m_viewer->overlay()->controlsBounds()));
                }
                const QImage image = window.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const QPoint viewerOrigin = window.m_viewer->mapTo(&window, QPoint());
                const auto at = [&](const QPointF &viewerPoint) {
                    return image.pixelColor((QPointF(viewerOrigin) + viewerPoint).toPoint() * ratio).rgb();
                };
                // Whether the 7 by 7 area at `viewerPoint` is the water: its
                // average a dark blue-grey (blue above red), as no stage,
                // card or panel is. Averaged, so the footage's grain and
                // ripples do not matter.
                const auto water = [&](const QPointF &viewerPoint) {
                    double red = 0, green = 0, blue = 0;
                    const QPoint centre = (QPointF(viewerOrigin) + viewerPoint).toPoint();
                    for (int dy = -3; dy <= 3; ++dy)
                        for (int dx = -3; dx <= 3; ++dx) {
                            const QColor c = image.pixelColor((centre + QPoint(dx, dy)) * ratio);
                            red += c.red(); green += c.green(); blue += c.blue();
                        }
                    red /= 49; green /= 49; blue /= 49;
                    return blue > red + 12 && (red + green + blue) / 3 < 110;
                };
                // Above the player's gradient, either side of the sun's path.
                QVERIFY2(water(QPointF(video.left() + 14, video.top() + video.height() * 0.7)), qPrintable(label));
                QVERIFY2(water(QPointF(video.right() - 14, video.top() + video.height() * 0.7)), qPrintable(label));
                // Around the group, the stage; between card and rail too.
                QCOMPARE(at(QPointF(body.left() - 6, body.top() + 8)), theme.viewerSurround.rgb());
                QCOMPARE(at(QPointF(body.right() + 6, body.top() + 8)), theme.viewerSurround.rgb());
                // The card's (or phone's) and the panel's corners are round:
                // the stage shows at the very corner.
                QCOMPARE(at(body.topLeft() + QPointF(1, 1)), theme.viewerSurround.rgb());
                if (tall) {
                    // The phone's graphite frame in either appearance, and its
                    // screen's black above the picture.
                    QCOMPARE(at(QPointF(body.center().x(), body.top() + stage.screen.top() - body.top()) - QPointF(0, 2)), theme.deviceFrame.rgb());
                    QCOMPARE(at(QPointF(stage.screen.left() + 3, (stage.screen.top() + video.top()) / 2.0 + 6)), theme.deviceScreen.rgb());
                }
                if (wide) {
                    // The laptop's stone rim and black glass in either
                    // appearance, the screen's black bar over the picture,
                    // and its base.
                    const LaptopGeometry laptop = laptopGeometry(body, 16.0 / 9.0);
                    QCOMPARE(at(QPointF(std::ceil(laptop.lid.left()) + 0.25, laptop.lid.center().y())), theme.laptopShell.rgb());
                    QCOMPARE(at(QPointF((laptop.lid.left() + laptop.rim + laptop.screen.left()) / 2.0, laptop.lid.center().y())), theme.laptopBezel.rgb());
                    QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.top() + video.top()) / 2.0)), theme.deviceScreen.rgb());
                    QCOMPARE(at(QPointF(laptop.base.left() + laptop.base.width() * 0.25, laptop.base.top() + 2.0)), theme.laptopDeck.rgb());
                }
                QCOMPARE(at(QPointF(panel.left() + 1, panel.top() + 1)), theme.viewerSurround.rgb());
                // The panel is its own surface, under its header.
                QCOMPARE(at(QPointF(panel.left() + 4, panel.top() + ChatPanel::kHeaderHeight / 2)), theme.chatSurface.rgb());
                QVERIFY(theme.chatSurface.rgb() != theme.viewerSurround.rgb());
                // Its header: the title, the count, ⋯ and the close button.
                QCOMPARE(window.m_chat->title()->text(), QStringLiteral("Kerf"));
                QCOMPARE(window.m_chat->countLabel()->text(), QStringLiteral("3"));
                QVERIFY(window.m_chat->moreButton()->isVisible() && window.m_chat->closeButton()->isVisible());
                QCOMPARE(window.m_rail->chatButton()->label(), QStringLiteral("3"));
                QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
                QVERIFY(window.m_chatToggle->isChecked() && window.m_rail->chatButton()->isChecked());
                // Kerf's steps folded under its comment.
                QCOMPARE(window.m_chat->rows().at(1)->expander()->text(), QStringLiteral("2 steps"));
                // Its next steps show once, in it, not again over the timeline.
                QVERIFY(window.m_chatNextSteps->isVisible() && !window.m_aiBar->isVisible());
                QCOMPARE(amberPixels(image), 0);
                const QString schemeName = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");
                if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("editor-%1-%2.png").arg(label, schemeName))));
                if (tall && !shots.isEmpty()) {
                    // The other phone, safe zones over it, and the plain card.
                    window.setDevicePreview(DevicePreview::PunchHole);
                    QCoreApplication::processEvents();
                    QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-9x16-punchhole-%1.png").arg(schemeName))));
                    window.setSafeZones(true);
                    QCoreApplication::processEvents();
                    QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-9x16-safezones-%1.png").arg(schemeName))));
                    window.setSafeZones(false);
                    window.setDevicePreview(DevicePreview::Off);
                    QCoreApplication::processEvents();
                    QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-9x16-card-%1.png").arg(schemeName))));
                    window.setDevicePreview(DevicePreview::Island);
                    QCoreApplication::processEvents();
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                if (wide) {
                    // The laptop Off: the plain card, level with the
                    // conversation and as tall.
                    window.setLaptopPreview(LaptopPreview::Off);
                    QCoreApplication::processEvents();
                    const ViewerWidget::StageLayout card = window.m_viewer->stageLayout();
                    QVERIFY(!card.device);
                    QCOMPARE(window.m_chat->geometry().top(), qRound(card.card.top()));
                    QVERIFY(std::abs(window.m_chat->height() - card.card.height()) <= 1);
                    const QImage off = window.grab().toImage();
                    QCOMPARE(amberPixels(off), 0);
                    if (!shots.isEmpty()) QVERIFY(off.save(QDir(shots).filePath(QStringLiteral("editor-16x9-off-%1.png").arg(schemeName))));
                    window.setLaptopPreview(LaptopPreview::Laptop);
                    QCoreApplication::processEvents();
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                // Closed from the toolbar, the picture (its phone, its
                // laptop) is centred alone, the rail hanging on its right.
                if (tall || wide) {
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(!window.m_chat->isVisible() && !window.m_chatToggle->isChecked());
                    // Closed, the timeline's bar stands in for it with the next steps.
                    QVERIFY(window.m_aiBar->isVisible());
                    const QRectF alone = window.m_viewer->stageLayout().body;
                    QVERIFY(std::abs(alone.center().x() - window.m_viewer->width() / 2.0) <= 1);
                    QVERIFY(std::abs(alone.height() - room.height()) < 1);
                    // The same inset over it as under it, the card's 16 for
                    // the laptop; the phone's only as much less as its
                    // 240-point screen needs here.
                    {
                        const double over = alone.top(), under = window.m_viewer->height() - alone.bottom();
                        QVERIFY2(std::abs(over - under) <= 1, qPrintable(QStringLiteral("%1: %2 over, %3 under").arg(label).arg(over).arg(under)));
                        QVERIFY2(std::abs(over - inset) <= 1, qPrintable(QStringLiteral("%1: %2 over for %3").arg(label).arg(over).arg(inset)));
                        if (tall) {
                            QVERIFY(inset >= ViewerWidget::kLeastStageInset && inset <= ViewerWidget::kStageMargin);
                            QVERIFY(window.m_viewer->stageLayout().screen.width() >= ViewerWidget::kLeastPhoneScreen);
                            if (inset < ViewerWidget::kStageMargin) {
                                // One more point over and under and the screen
                                // would fall under 240.
                                const double body = window.m_viewer->height() - 2.0 * (inset + 1);
                                QVERIFY(body * deviceBodyAspect() * (1.0 - 2.0 * kDeviceBezel) < ViewerWidget::kLeastPhoneScreen);
                            }
                        }
                    }
                    QVERIFY(window.m_rail->isVisible());
                    QCOMPARE(window.m_rail->x() - qRound(alone.right()), window.m_viewer->stageGap());
                    const QImage closed = window.grab().toImage();
                    QCOMPARE(amberPixels(closed), 0);
                    if (!shots.isEmpty()) QVERIFY(closed.save(QDir(shots).filePath(QStringLiteral("editor-%1-closed-%2.png").arg(label, schemeName))));
                    // Opened again, the group is back as it was.
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(window.m_chat->isVisible());
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                window.close();
            }
        }
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
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
        });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QString schemeName = theme.dark ? QStringLiteral("dark") : QStringLiteral("light");
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
                const QList<QPair<QString, QString>> expected{{QStringLiteral("Trim fisher-close.mov"), QStringLiteral("end \u22121.5 s")},
                                                              {QStringLiteral("Remove drone-pass.mov"), QStringLiteral("0:11\u2009\u2013\u20090:18")},
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
        // Footage (the harbor at dawn: sky, hills and water), on which the
        // masks' outlines and names must read.
        const QImage picture = testing::footageFrame(testing::Shot::HarborWide, QSize(960, 540));
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
        // The preview as Edge Glow would draw it on the harbor shot: the
        // frame with a soft, blurred copy of itself screened over it.
        QImage glow = testing::footageFrame(testing::Shot::HarborWide, QSize(160, 90)).convertToFormat(QImage::Format_RGB32);
        {
            const QImage soft = glow.scaled(20, 11, Qt::IgnoreAspectRatio, Qt::SmoothTransformation)
                                    .scaled(glow.size(), Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
            QPainter painter(&glow);
            painter.setCompositionMode(QPainter::CompositionMode_Screen);
            painter.setOpacity(0.6);
            painter.drawImage(0, 0, soft);
        }
        effects.setPreview(QStringLiteral("c1"), QStringLiteral("light.edgeglow"), glow);
        effects.resize(420, 200);
        const auto tilePixel = [&bin](int row, const QPoint &at) {
            return bin.m_list->item(row)->icon().pixmap(ProjectPanel::kThumbnail).toImage().pixelColor(at);
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
            // The selected mask's name is light over the picture.
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
            // Empty, the picture is the laptop's black screen (the plain card's canvas without one).
            QCOMPARE(empty.pixelColor((viewer.videoRect().topLeft() + QPointF(6, 6)).toPoint() * ratio),
                     viewer.stageLayout().device ? theme.deviceScreen : theme.viewerCanvas);
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
            QCOMPARE(cardPixel("light.edgeglow").rgb(), glow.pixelColor(glow.width() / 2, 4).rgb());
            effects.show();
            QVERIFY(QTest::qWaitForWindowExposed(&effects));
            // Nothing under the pointer.
            QTest::mouseMove(&effects, QPoint(effects.width() - 2, effects.height() - 2));
            effects.m_category->setAttribute(Qt::WA_UnderMouse, false);
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
