// Tests for the media bin, artwork, effects and presets browsers.

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

class BinBrowserTest final : public QObject {
    Q_OBJECT

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

    // Media is named as it was imported: the content hash media.rs gives the
    // copy, and the extension, stay out of the bin, timeline and Inspector.
    void mediaIsNamedWithoutItsContentHash() {
        const auto named = [](const char *path) { return sourceName(QJsonObject{{"path", QString::fromUtf8(path)}}); };
        QCOMPARE(named("media/Beach Day-1a2b3c4d.mov"), QStringLiteral("Beach Day"));
        QCOMPARE(named("media/vertical_clip-7e0b5b62.mp4"), QStringLiteral("vertical_clip"));
        QCOMPARE(named("media/clip.v2-0f0f0f0f.mp4"), QStringLiteral("clip.v2"));
        QCOMPARE(named("media/새벽 능선-a1b2c3d4.mov"), QStringLiteral("새벽 능선"));
        // Not a content hash: too short, upper case, or all there is.
        QCOMPARE(named("media/take-12.mov"), QStringLiteral("take-12"));
        QCOMPARE(named("media/take-ABCDEF12.mov"), QStringLiteral("take-ABCDEF12"));
        QCOMPARE(named("media/-1a2b3c4d.mov"), QStringLiteral("-1a2b3c4d"));
        QCOMPARE(named(""), QStringLiteral("Untitled media"));
        QCOMPARE(sourceFileName(QJsonObject{{"path", "media/Beach Day-1a2b3c4d.mov"}}), QStringLiteral("Beach Day.mov"));
        QCOMPARE(sourceFileName(QJsonObject{{"path", "media/notes-1a2b3c4d"}}), QStringLiteral("notes"));
        // Media that would read alike is told apart: by extension first,
        // then by the copy's own name.
        const QHash<QString, QString> names = sourceNames(QJsonArray{
            QJsonObject{{"id", "a"}, {"path", "media/Beach Day-1a2b3c4d.mov"}},
            QJsonObject{{"id", "b"}, {"path", "media/Beach Day-5e6f7a8b.wav"}},
            QJsonObject{{"id", "c"}, {"path", "media/Interview-aaaa1111.mov"}},
            QJsonObject{{"id", "d"}, {"path", "media/Interview-bbbb2222.mov"}},
            QJsonObject{{"id", "e"}, {"path", "media/harbor-0000ffff.mov"}}});
        QCOMPARE(names.value("a"), QStringLiteral("Beach Day.mov"));
        QCOMPARE(names.value("b"), QStringLiteral("Beach Day.wav"));
        QCOMPARE(names.value("c"), QStringLiteral("Interview-aaaa1111.mov"));
        QCOMPARE(names.value("d"), QStringLiteral("Interview-bbbb2222.mov"));
        QCOMPARE(names.value("e"), QStringLiteral("harbor"));
        // Hundredths carry into the seconds and minutes.
        QCOMPARE(prettyTime(59.996), QStringLiteral("01:00.00"));
        QCOMPARE(prettyTime(8.0), QStringLiteral("00:08.00"));
        QCOMPARE(prettyTime(61.25), QStringLiteral("01:01.25"));
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
        // The sound as peaks: 0.2 s of FFmpeg's sine, at its 1/8 amplitude.
        const AudioPeaks peaks = artwork.peaks.value(QStringLiteral("tone"));
        QVERIFY(std::abs(peaks.duration() - 0.2) < 0.02);
        QVERIFY(std::abs(peaks.range(0.05, 0.15).max - 0.125f) < 0.01f);
    }

    // A vertical clip's poster, filmstrip and peaks, from FFmpeg: the poster
    // and every strip frame at the picture's own 9:16 shape, no bars baked
    // in; the poster covers the bin's tile at 2x, the strip is its height.
    void artworkKeepsEveryPictureAtItsShape() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        const QString path = directory.filePath(QStringLiteral("vertical.mov"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {"-v", "error", "-f", "lavfi", "-i", "testsrc2=size=540x960:rate=25:duration=2",
                                                              "-f", "lavfi", "-i", "sine=frequency=220:duration=2", "-c:v", "mpeg4",
                                                              "-q:v", "4", "-c:a", "pcm_s16le", "-shortest", path}), 0);
        MediaArtwork artwork;
        artwork.request(QJsonArray{QJsonObject{{"id", "v"}, {"path", path}, {"width", 540}, {"height", 960}, {"duration", 2.0},
                                               {"hasAudio", true}}}, directory.path());
        // Strips come last, after every poster and sound.
        QTRY_VERIFY_WITH_TIMEOUT(artwork.strips.contains(QStringLiteral("v")), 20000);
        QVERIFY(artwork.frames.contains(QStringLiteral("v")) && artwork.peaks.contains(QStringLiteral("v")));
        const QImage poster = artwork.frames.value(QStringLiteral("v"));
        QVERIFY2(std::abs(double(poster.width()) / poster.height() - 9.0 / 16.0) < 0.02,
                 qPrintable(QStringLiteral("%1x%2").arg(poster.width()).arg(poster.height())));
        QVERIFY(poster.width() >= MediaArtwork::kPosterSize.width() && poster.height() >= MediaArtwork::kPosterSize.height());
        const Filmstrip strip = artwork.strips.value(QStringLiteral("v"));
        QVERIFY2(strip.frames.size() >= 12 && strip.frames.size() <= MediaArtwork::stripFrameCount(2.0),
                 qPrintable(QString::number(strip.frames.size())));
        QCOMPARE(strip.frames.first().height(), MediaArtwork::kStripHeight);
        QVERIFY(std::abs(strip.aspect() - 9.0 / 16.0) < 0.03);
        QVERIFY(std::abs(strip.interval - 2.0 / MediaArtwork::stripFrameCount(2.0)) < 1e-9);
        // testsrc2 moves: the strip's frames are not one picture repeated.
        QVERIFY(strip.frames.first() != strip.frames.last());
        QVERIFY(artwork.peaks.value(QStringLiteral("v")).range(0.5, 1.5).max > 0.1f);
    }

    // A long source's strip is sought a frame at a time; an id that comes
    // back to a file it named before (an undone relink) has that file's
    // artwork again, not the other's; media taken out lets go of its own.
    void artworkFollowsRelinksAndRemovals() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        const QString longPath = directory.filePath(QStringLiteral("long.mov"));
        const QString shortPath = directory.filePath(QStringLiteral("short.mov"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {"-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x284:rate=10:duration=17",
                                                              "-c:v", "mpeg4", "-q:v", "6", longPath}), 0);
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {"-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=2",
                                                              "-c:v", "mpeg4", "-q:v", "6", shortPath}), 0);
        const auto source = [](const QString &path, double duration) {
            return QJsonArray{QJsonObject{{"id", "v"}, {"path", path}, {"width", 160}, {"height", 284}, {"duration", duration}}};
        };
        MediaArtwork artwork;
        artwork.request(source(longPath, 17.0), directory.path());
        QTRY_VERIFY_WITH_TIMEOUT(artwork.strips.contains(QStringLiteral("v")), 60000);
        Filmstrip strip = artwork.strips.value(QStringLiteral("v"));
        QCOMPARE(strip.frames.size(), MediaArtwork::kStripFrames);
        QVERIFY(std::abs(strip.interval - 17.0 / MediaArtwork::kStripFrames) < 1e-9);
        QVERIFY(std::abs(strip.aspect() - 160.0 / 284.0) < 0.03);
        QVERIFY(strip.frames.first() != strip.frames.last());
        // Relinked to a short, wide file: its own strip.
        artwork.request(source(shortPath, 2.0), directory.path());
        QVERIFY(!artwork.strips.contains(QStringLiteral("v")));
        QTRY_VERIFY_WITH_TIMEOUT(artwork.strips.contains(QStringLiteral("v")), 20000);
        QVERIFY(artwork.strips.value(QStringLiteral("v")).aspect() > 1.5);
        // Back to the long file: asked for again, so its strip is its own.
        artwork.request(source(longPath, 17.0), directory.path());
        QVERIFY(!artwork.strips.contains(QStringLiteral("v")));
        QTRY_VERIFY_WITH_TIMEOUT(artwork.strips.contains(QStringLiteral("v")), 60000);
        QVERIFY(artwork.strips.value(QStringLiteral("v")).aspect() < 1.0);
        // Taken out of the project: nothing kept for it.
        artwork.request(QJsonArray{}, directory.path());
        QVERIFY(!artwork.strips.contains(QStringLiteral("v")) && !artwork.frames.contains(QStringLiteral("v")));
    }

    // Sound reduces, as it streams in, to 5 ms buckets of its extremes and
    // RMS; each coarser level merges four; a span reads the coarsest level
    // that still fits it, and a span inside one bucket is interpolated.
    void audioPeaksReduceAndMergeSound() {
        // One second of a 440 Hz sine at half scale, in uneven chunks.
        QVector<float> sine(16000);
        for (int index = 0; index < sine.size(); ++index) sine[index] = 0.5f * float(std::sin(2.0 * 3.14159265358979 * 440.0 * index / 16000.0));
        PeakReducer reducer(16000);
        for (int at = 0; at < sine.size(); at += 333) reducer.add(sine.constData() + at, std::min<qsizetype>(333, sine.size() - at));
        const AudioPeaks peaks = reducer.finish();
        QCOMPARE(peaks.levels.first().size(), qsizetype(AudioPeaks::kBucketsPerSecond));
        QCOMPARE(peaks.duration(), 1.0);
        // A waveform is raised until the loudest peak nearly fills its clip,
        // a quiet recording by at most 12 dB, silence not at all.
        QVERIFY(std::abs(peaks.displayGain() - 0.95 / 0.5) < 0.01);
        QVector<float> whisper(sine);
        for (float &sample : whisper) sample *= 0.02f;
        PeakReducer quiet(16000);
        quiet.add(whisper.constData(), whisper.size());
        QCOMPARE(quiet.finish().displayGain(), 4.0);
        PeakReducer hush(16000);
        const QVector<float> silence(16000, 0.0f);
        hush.add(silence.constData(), silence.size());
        QCOMPARE(hush.finish().displayGain(), 1.0);
        // A quiet take with its gain raised fills its clip without being cut
        // flat at the top, as only sound over full scale is.
        QVector<float> soft(sine);
        for (float &sample : soft) sample *= 0.4f;
        PeakReducer softer(16000);
        softer.add(soft.constData(), soft.size());
        const AudioPeaks raised = softer.finish();
        const auto topRowInk = [&raised](double gain) {
            QImage image(200, 40, QImage::Format_ARGB32_Premultiplied);
            image.fill(Qt::transparent);
            QPainter painter(&image);
            drawWaveform(painter, QRectF(image.rect()), raised, 0.0, 1.0, Qt::black, Qt::black, gain, QRectF());
            painter.end();
            int ink = 0;
            for (int x = 0; x < image.width(); ++x) ink = std::max(ink, qAlpha(image.pixel(x, 0)));
            return ink;
        };
        QVERIFY2(topRowInk(2.0) < 128, qPrintable(QString::number(topRowInk(2.0))));
        // Truly over full scale (0.2 at +18 dB), it is cut flat.
        QCOMPARE(topRowInk(8.0), 255);
        for (const AudioPeaks::Bucket &bucket : peaks.levels.first()) {
            QVERIFY(std::abs(bucket.max - 0.5f) < 0.005f && std::abs(bucket.min + 0.5f) < 0.005f);
            QVERIFY(std::abs(bucket.rms - 0.3536f) < 0.03f);
        }
        // 200, 50, 13, 4, 1.
        QList<qsizetype> sizes;
        for (const auto &level : peaks.levels) sizes << level.size();
        QCOMPARE(sizes, (QList<qsizetype>{200, 50, 13, 4, 1}));
        const AudioPeaks::Bucket whole = peaks.range(0.0, 1.0);
        QVERIFY(std::abs(whole.max - 0.5f) < 0.005f && std::abs(whole.rms - 0.3536f) < 0.01f);
        // Past the end is silence.
        QCOMPARE(peaks.range(2.0, 3.0).max, 0.0f);

        // Merging: the extremes of the four, and the RMS of their RMS.
        const AudioPeaks::Bucket four[] = {{-0.2f, 0.4f, 0.3f}, {-0.6f, 0.1f, 0.1f}, {-0.1f, 0.2f, 0.1f}, {-0.3f, 0.7f, 0.5f}};
        const AudioPeaks::Bucket merged = AudioPeaks::merge(four, 4);
        QCOMPARE(merged.min, -0.6f);
        QCOMPARE(merged.max, 0.7f);
        QVERIFY(std::abs(merged.rms - float(std::sqrt((0.09 + 0.01 + 0.01 + 0.25) / 4.0))) < 1e-6f);

        // A ramp: bucket i peaks at i/100.
        AudioPeaks ramp;
        ramp.levels.append(QVector<AudioPeaks::Bucket>(200));
        for (int index = 0; index < 200; ++index) ramp.levels[0][index] = {0.0f, index / 100.0f, index / 200.0f};
        ramp.buildLevels();
        QCOMPARE(ramp.levels.at(1).at(0).max, 0.03f);
        // Exactly one level-1 bucket's span reads that bucket.
        QCOMPARE(ramp.range(0.0, AudioPeaks::bucketSeconds(1)).max, 0.03f);
        // A minute-long span reads a few top-level buckets, not 12,000.
        QCOMPARE(ramp.range(0.0, 60.0).max, 1.99f);
        // Inside one bucket: between the buckets either side of its middle
        // (bucket centres sit half a bucket in).
        const AudioPeaks::Bucket between = ramp.range(0.10255, 0.10256);
        QVERIFY2(std::abs(between.max - 0.2001f) < 0.0005f, qPrintable(QString::number(between.max)));
    }

    // A filmstrip tile shows the frame nearest the source time under its
    // left edge, the clip's in point first; speed and reversal map the
    // clip's time to its source's.
    void filmstripPicksTheFrameUnderEachTile() {
        Filmstrip strip;
        QCOMPARE(strip.frameAt(1.0), -1);
        strip.interval = 0.5;
        for (int index = 0; index < 10; ++index) strip.frames << QImage(54, 96, QImage::Format_RGB32);
        QCOMPARE(strip.frameAt(0.0), 0);
        QCOMPARE(strip.frameAt(0.24), 0);
        QCOMPARE(strip.frameAt(0.26), 1);
        QCOMPARE(strip.frameAt(2.0), 4);
        QCOMPARE(strip.frameAt(-3.0), 0);
        QCOMPARE(strip.frameAt(100.0), 9);
        QCOMPARE(strip.aspect(), 54.0 / 96.0);
        const auto at = [](const QJsonObject &clip, double offset) { return TimelineWidget::clipSourceTime(clip, offset); };
        QCOMPARE(at(QJsonObject{{"in", 2.0}, {"out", 6.0}, {"duration", 2.0}, {"speed", 2.0}}, 1.0), 4.0);
        QCOMPARE(at(QJsonObject{{"in", 1.0}, {"duration", 2.0}}, 0.5), 1.5);
        QCOMPARE(at(QJsonObject{{"in", 1.0}, {"out", 3.0}, {"duration", 2.0}, {"speed", -1.0}}, 0.0), 3.0);
        QCOMPARE(at(QJsonObject{{"in", 1.0}, {"out", 3.0}, {"duration", 2.0}, {"speed", -1.0}}, 2.0), 1.0);
        // As the bridge sends a reversed clip: the speed's size and "reverse".
        QCOMPARE(at(QJsonObject{{"in", 1.0}, {"out", 3.0}, {"duration", 2.0}, {"speed", 1.0}, {"reverse", true}}, 0.0), 3.0);
        QCOMPARE(at(QJsonObject{{"in", 1.0}, {"out", 3.0}, {"duration", 2.0}, {"speed", 1.0}, {"reverse", true}}, 0.5), 2.5);
        QCOMPARE(at(QJsonObject{{"in", 0.0}, {"out", 4.0}, {"duration", 2.0}, {"speed", 2.0}, {"reverse", true}}, 1.0), 2.0);
    }

    // Bin tiles fit a poster at its own shape, the bars in the tile's colour
    // (not black from the picture), and draw a sound's waveform from its peaks.
    void binTilesFitPostersAtTheirShape() {
        MediaArtwork artwork;
        QImage vertical(79, 140, QImage::Format_RGB32);
        vertical.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("tall"), vertical);
        artwork.peaks.insert(QStringLiteral("snd"), fullScalePeaks(4.0));
        ProjectPanel bin(&artwork);
        bin.setSources(QJsonArray{
            QJsonObject{{"id", "tall"}, {"path", "media/tall.mov"}, {"width", 1080}, {"height", 1920}, {"duration", 4}},
            QJsonObject{{"id", "snd"}, {"path", "media/voice.wav"}, {"duration", 4}, {"hasAudio", true}}});
        const QImage tall = bin.m_list->item(0)->icon().pixmap(ProjectPanel::kThumbnail).toImage();
        const QPoint centre(ProjectPanel::kThumbnail.width() / 2, ProjectPanel::kThumbnail.height() / 2);
        QCOMPARE(tall.pixelColor(centre), QColor("#e87342"));
        // 9:16 in a 70px tall tile is 39px wide: bars either side.
        QCOMPARE(tall.pixelColor(centre.x() - 22, centre.y()), editorTheme().thumbnail);
        QCOMPARE(tall.pixelColor(centre.x() + 22, centre.y()), editorTheme().thumbnail);
        QCOMPARE(tall.pixelColor(centre.x() - 18, centre.y()), QColor("#e87342"));
        const QImage sound = bin.m_list->item(1)->icon().pixmap(ProjectPanel::kThumbnail).toImage();
        QVERIFY(isWaveInk(sound.pixelColor(centre.x(), centre.y() + 10)));
        QCOMPARE(sound.pixelColor(2, centre.y()), editorTheme().audioTile);
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

KERF_TEST_CLASS(kerf::BinBrowserTest)

#include "bin_browser_test.moc"
