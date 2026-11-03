// Tests for the stage rail and the program level meter.

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

class ScrubRailTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // The rail's head holds the program's VU meter, as wide as the rail and
    // level with the picture's top, over the buttons at its foot; at
    // 1366x768 it fits beside a phone (Phone, Safe, Chat) and a laptop
    // (Laptop, Chat) with the chat open, in both appearances. Its needle
    // reads the sound under the playhead while the program plays, and
    // falls back to its stop, its timer stopped, at rest. With
    // KERF_SCREENSHOT_DIR set, each stage is saved as
    // stage-rail-<format>-<scheme>.png.
    void stageRailHoldsTheProgramLevel() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] {
            setEditorColorScheme(ColorScheme::Dark);
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("viewer/chatOpen"));
            QSettings().remove(QStringLiteral("viewer/chatWidth"));
        });
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const QString name = editorTheme().dark ? QStringLiteral("dark") : QStringLiteral("light");
            for (const QSize canvas : {QSize(1080, 1920), QSize(1920, 1080)}) {
                const bool tall = canvas.height() > canvas.width();
                QSettings().remove(QStringLiteral("workspace"));
                QSettings().remove(QStringLiteral("viewer/chatOpen"));
                QSettings().remove(QStringLiteral("viewer/chatWidth"));
                MainWindow window{QString(), QString()};
                window.setConnection(QStringLiteral("ready"));
                QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
                snapshot.remove(QStringLiteral("pendingProposal"));
                snapshot.remove(QStringLiteral("proposalTimeline"));
                QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
                timeline.insert(QStringLiteral("width"), canvas.width());
                timeline.insert(QStringLiteral("height"), canvas.height());
                snapshot.insert(QStringLiteral("timeline"), timeline);
                window.applySnapshot(snapshot);
                fakeArtwork(window.m_artwork);
                window.resize(1366, 768);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                if (!window.chatOpen()) window.m_chatToggle->click();
                QCoreApplication::processEvents();
                const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
                QVERIFY(!stage.overlay && !stage.rail.isEmpty());
                StageRail *rail = window.m_rail;
                QCOMPARE(rail->geometry(), stage.rail);
                VuMeter *meter = rail->meter();
                QVERIFY2(meter->isVisible(), qPrintable(QStringLiteral("rail %1 tall").arg(rail->height())));
                QCOMPARE(meter->geometry(), QRect(0, 0, StageRail::kWidth, StageRail::kMeterHeight));
                QCOMPARE(meter->accessibleName(), QStringLiteral("Program level"));
                QCOMPARE(meter->toolTip(), QStringLiteral("Program level"));
                // The buttons at the foot, whole, clear of the meter.
                QList<RailButton *> buttons{rail->deviceButton(), rail->chatButton()};
                if (tall) buttons.insert(1, rail->safeZonesButton());
                QCOMPARE(rail->safeZonesButton()->isVisible(), tall);
                for (RailButton *button : buttons) {
                    QVERIFY(button->isVisible());
                    QVERIFY(rail->rect().contains(button->geometry()));
                    QVERIFY(button->geometry().top() >= meter->geometry().bottom() + StageRail::kMeterGap);
                }
                QCOMPARE(rail->chatButton()->geometry().bottom(), rail->height() - 1);

                // At rest: the left stop, no timer.
                QCOMPARE(meter->needle(), 0.0);
                QVERIFY(!meter->animating());
                window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", true}});
                window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 3.0}, {"duration", 18.0}});
                const double reading = vuNeedle(programPeak(window.m_project, window.m_artwork, 3.0));
                QVERIFY(reading > 0.0);
                QCOMPARE(meter->target(), reading);
                QVERIFY(meter->animating());
                QTRY_VERIFY(std::abs(meter->needle() - reading) < 0.01);
                // Playing on, the timer keeps running for the next reading.
                QVERIFY(meter->animating());
                if (!shots.isEmpty())
                    QVERIFY(window.m_viewer->grab().toImage().save(QDir(shots).filePath(
                        QStringLiteral("stage-rail-%1-%2.png").arg(tall ? QStringLiteral("9x16") : QStringLiteral("16x9"), name))));
                window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", false}});
                QCOMPARE(meter->target(), 0.0);
                QTRY_VERIFY(meter->needle() == 0.0 && !meter->animating());
                window.close();
            }
        }
    }

    // The program's level is the loudest peak of the sound clips switched on under the
    // playhead on enabled, unmuted sound tracks, each with its gain, read at
    // its own source time; the VU scale puts -18 dBFS at 0 VU, three
    // quarters of the way across, -20 VU at the left stop and +3 VU at the
    // right.
    void programLevelReadsTheSoundUnderThePlayhead() {
        MediaArtwork artwork;
        // A second at a quarter of full scale, then three of silence.
        AudioPeaks peaks;
        peaks.levels.append(QVector<AudioPeaks::Bucket>(4 * AudioPeaks::kBucketsPerSecond));
        for (int index = 0; index < AudioPeaks::kBucketsPerSecond; ++index) peaks.levels[0][index] = {-0.25f, 0.2f, 0.1f};
        peaks.buildLevels();
        artwork.peaks.insert(QStringLiteral("tone"), peaks);
        const auto clip = [](const char *id, double start, double in, const QJsonObject &props = {}) {
            return QJsonObject{{"id", id}, {"sourceId", "tone"}, {"timelineStart", start}, {"duration", 2.0},
                               {"in", in}, {"out", in + 2.0}, {"props", props}};
        };
        const auto timelineOf = [](const QJsonArray &tracks) { return QJsonObject{{"tracks", tracks}}; };
        const auto track = [](const char *type, const QJsonArray &clips, const QJsonObject &extra = {}) {
            QJsonObject out{{"id", type}, {"type", type}, {"clips", clips}};
            for (auto it = extra.begin(); it != extra.end(); ++it) out.insert(it.key(), it.value());
            return out;
        };
        const QJsonObject plain = timelineOf({track("audio", {clip("a", 2.0, 0.0)})});
        QCOMPARE(programPeak(plain, &artwork, 2.5), 0.25);
        // Before the clip, after its sound, and with no artwork: silence.
        QCOMPARE(programPeak(plain, &artwork, 1.0), 0.0);
        QCOMPARE(programPeak(plain, &artwork, 3.5), 0.0);
        QCOMPARE(programPeak(plain, nullptr, 2.5), 0.0);
        // Its in point decides which of the source plays.
        QCOMPARE(programPeak(timelineOf({track("audio", {clip("a", 2.0, 1.0)})}), &artwork, 2.5), 0.0);
        // Its gain, in dB.
        QVERIFY(qFuzzyCompare(programPeak(timelineOf({track("audio", {clip("a", 2.0, 0.0, {{"gain", 6.0}})})}), &artwork, 2.5),
                              0.25 * std::pow(10.0, 6.0 / 20.0)));
        // A muted or disabled sound track, or a picture track, sounds nothing.
        QCOMPARE(programPeak(timelineOf({track("audio", {clip("a", 2.0, 0.0)}, {{"muted", true}})}), &artwork, 2.5), 0.0);
        QCOMPARE(programPeak(timelineOf({track("audio", {clip("a", 2.0, 0.0)}, {{"enabled", false}})}), &artwork, 2.5), 0.0);
        QCOMPARE(programPeak(timelineOf({track("video", {clip("a", 2.0, 0.0)})}), &artwork, 2.5), 0.0);
        // So does a clip switched off on its own.
        QJsonObject off = clip("a", 2.0, 0.0);
        off.insert("enabled", false);
        QCOMPARE(programPeak(timelineOf({track("audio", {off})}), &artwork, 2.5), 0.0);
        // Two tracks: the louder.
        const QJsonObject both = timelineOf({track("audio", {clip("a", 2.0, 0.0, {{"gain", -12.0}})}),
                                             track("audio", {clip("b", 2.0, 0.0)})});
        QCOMPARE(programPeak(both, &artwork, 2.5), 0.25);

        QCOMPARE(vuNeedle(0.0), 0.0);
        QCOMPARE(vuNeedle(std::pow(10.0, -40.0 / 20.0)), 0.0);
        QVERIFY(std::abs(vuNeedle(std::pow(10.0, -38.0 / 20.0))) < 1e-9);
        QVERIFY(std::abs(vuNeedle(std::pow(10.0, -18.0 / 20.0)) - 0.75) < 1e-9);
        QVERIFY(std::abs(vuNeedle(std::pow(10.0, -15.0 / 20.0)) - 1.0) < 1e-9);
        QCOMPARE(vuNeedle(1.0), 1.0);
        // Rising steadily between, crowded at the quiet end.
        double last = 0.0;
        for (double dbfs = -37.0; dbfs <= -15.0; dbfs += 1.0) {
            const double needle = vuNeedle(std::pow(10.0, dbfs / 20.0));
            QVERIFY(needle > last);
            last = needle;
        }
        QVERIFY(vuNeedle(std::pow(10.0, -28.0 / 20.0)) < 0.25);
    }

    // The needle has a VU meter's ballistics: 99% of a step in about 300 ms,
    // up and down alike; its timer runs only while the program plays or the
    // needle is settling.
    void vuMeterMovesLikeAVuMeter() {
        VuMeter meter;
        QCOMPARE(meter.needle(), 0.0);
        QVERIFY(!meter.animating());
        meter.setTarget(1.0);
        QVERIFY(meter.animating());
        meter.advance(0.1);
        QVERIFY2(meter.needle() > 0.6 && meter.needle() < 0.9, qPrintable(QString::number(meter.needle())));
        meter.advance(0.2);
        QVERIFY(meter.needle() >= 0.99 && meter.needle() <= 1.0);
        meter.setTarget(0.0);
        meter.advance(VuMeter::kSettleSeconds);
        QVERIFY(meter.needle() <= 0.01);
        // Readings outside the scale stop at its ends.
        meter.setTarget(4.0);
        QCOMPARE(meter.target(), 1.0);
        meter.setTarget(-1.0);
        QCOMPARE(meter.target(), 0.0);
        // Left alone it settles, and its timer stops.
        meter.setTarget(0.6);
        QTRY_VERIFY(meter.needle() == 0.6 && !meter.animating());
        // Playing, it keeps ticking for the next reading even when still.
        meter.setPlaying(true);
        QVERIFY(meter.animating());
        meter.setPlaying(false);
        QVERIFY(!meter.animating());
    }

    // A rail button's disc is a raised key of the hardware finish: off, it
    // stands proud in the key's shading; on, it is held in and its amber
    // LED is lit at its top, in both appearances. Its glyph reads on the
    // key either way.
    void railButtonLightsItsLedWhenOn() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        const auto distance = [](const QColor &a, const QColor &b) {
            return std::abs(a.red() - b.red()) + std::abs(a.green() - b.green()) + std::abs(a.blue() - b.blue());
        };
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &t = editorTheme();
            const QString name = t.dark ? QStringLiteral("dark") : QStringLiteral("light");
            for (const QColor &key : {t.keyTop, t.keyBottom, t.keyLatched})
                QVERIFY2(contrastRatio(t.railGlyph, key) >= 3.0, qPrintable(name + QStringLiteral(" glyph on ") + key.name()));
            for (const QColor &key : {t.keyTop, t.keyBottom})
                QVERIFY2(contrastRatio(t.textSecondary, key) >= 3.0, qPrintable(name + QStringLiteral(" off glyph on ") + key.name()));
            RailButton button(RailButton::Glyph::SafeZones);
            button.setCheckable(true);
            button.setLabel(QStringLiteral("Safe"));
            button.setFixedSize(button.sizeHint());
            const QRect disc = button.discRect();
            // The LED's middle (a held key's face is a line lower), and the
            // face under the glyph.
            const QPoint led(disc.center().x() + 1, disc.top() + 7);
            const QPoint face(disc.center().x(), disc.bottom() - 5);
            // In the device pixels the grab holds, at any scale.
            const auto at = [](const QImage &image, const QPoint &point) {
                return image.pixelColor((QPointF(point) * image.devicePixelRatio()).toPoint());
            };
            const QImage off = button.grab().toImage();
            QVERIFY2(!isAmber(at(off, led)), qPrintable(name + QStringLiteral(" off ") + at(off, led).name()));
            QVERIFY(distance(at(off, face), t.keyBottom) < distance(at(off, face), t.keyLatched));
            button.setChecked(true);
            const QImage on = button.grab().toImage();
            QVERIFY2(isAmber(at(on, led)), qPrintable(name + QStringLiteral(" on ") + at(on, led).name()));
            QVERIFY(distance(at(on, led), t.keyLed) < 40);
            QVERIFY(distance(at(on, face), t.keyLatched) < distance(at(on, face), t.keyBottom));
            // The label stays under the disc.
            QCOMPARE(button.label(), QStringLiteral("Safe"));
            // A rail too short for the meter over its buttons keeps the
            // buttons; with one button fewer, the meter fits again.
            {
                // In a stage, as the viewer holds it: a window would keep
                // to its layout's least height.
                QWidget stage;
                auto *rail = new StageRail(&stage);
                stage.resize(200, 400);
                stage.show();
                const int buttons = 3 * rail->chatButton()->height() + 2 * rail->layout()->spacing();
                rail->resize(StageRail::kWidth, buttons + StageRail::kMeterGap + StageRail::kMeterHeight);
                QVERIFY(!rail->meter()->isHidden());
                // Just fitting, the meter sits kMeterGap over the first button
                // and the last ends at the rail's foot.
                QCoreApplication::sendPostedEvents(rail, QEvent::LayoutRequest);
                QCOMPARE(rail->deviceButton()->geometry().top() - rail->meter()->geometry().bottom() - 1, StageRail::kMeterGap);
                QCOMPARE(rail->chatButton()->geometry().bottom(), rail->height() - 1);
                rail->resize(StageRail::kWidth, buttons + StageRail::kMeterGap + StageRail::kMeterHeight - 1);
                QVERIFY(rail->meter()->isHidden());
                rail->safeZonesButton()->hide();
                QCoreApplication::sendPostedEvents(rail, QEvent::LayoutRequest);
                QVERIFY(!rail->meter()->isHidden());
            }
            // With KERF_SCREENSHOT_DIR set: a phone switched off (slashed),
            // safe zones off, and the chat on, saved as rail-keys-<scheme>.png.
            if (const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR"); !shots.isEmpty()) {
                QWidget stage;
                auto *rail = new StageRail(&stage);
                rail->setGeometry(0, 0, StageRail::kWidth, 300);
                rail->deviceButton()->setChecked(false);
                rail->chatButton()->setChecked(true);
                rail->chatButton()->setLabel(QStringLiteral("Chat"));
                // On the stage's own colour, as it shows beside the picture.
                const QPixmap keys = rail->grab();
                QImage image(keys.size() + QSize(32, 32) * keys.devicePixelRatio(), QImage::Format_ARGB32_Premultiplied);
                image.setDevicePixelRatio(keys.devicePixelRatio());
                image.fill(t.viewerSurround);
                QPainter(&image).drawPixmap(16, 16, keys);
                QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("rail-keys-%1.png").arg(name))));
            }
        }
    }

    // The VU's needle stands where its level puts it: hard left at 0, hard
    // right at 1, and no further than either however loud.
    void hardwareVuNeedleFollowsTheLevel() {
        const QRectF rect(0, 0, 120, 70);
        // The scale's geometry: the face inset 5px, the pivot near its foot,
        // the scale 50° either side of straight up.
        const QRectF face = rect.adjusted(5, 5, -5, -5);
        const QPointF pivot(face.center().x(), face.bottom() - face.height() * 0.08);
        const qreal reach = std::min(face.height() * 0.78, face.width() * 0.5 / std::sin(qDegreesToRadians(50.0)));
        const auto darkestAlong = [&](const QImage &image, double degrees) {
            int darkest = 255;
            const qreal radians = qDegreesToRadians(degrees);
            for (qreal along = 0.3; along <= 0.7; along += 0.05) {
                const QPointF at = pivot + QPointF(std::sin(radians), -std::cos(radians)) * reach * along;
                darkest = std::min(darkest, lightnessAt(image, at));
            }
            return darkest;
        };
        const auto meter = [&](double level) {
            return paintedAt(2, QSize(120, 70), [&](QPainter &p) { paintVuMeter(p, rect, level); });
        };
        const QImage quiet = meter(0);
        const QImage loud = meter(1);
        QVERIFY(darkestAlong(quiet, -50) < 80);
        QVERIFY(darkestAlong(quiet, 50) > 150);
        QVERIFY(darkestAlong(loud, 50) < 80);
        QVERIFY(darkestAlong(loud, -50) > 150);
        QVERIFY(darkestAlong(meter(0.5), 0) < 80);
        QCOMPARE(meter(2), loud);
        QCOMPARE(meter(-1), quiet);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ScrubRailTest)

#include "scrub_rail_test.moc"
