// Tests for the media bin, artwork, effects and presets browsers.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "ui/browser_panel.h"
#include "ui/components.h"
#include "ui/wiring/bin_wiring.h"

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
#include <QDragEnterEvent>
#include <QDropEvent>
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
#include <QShortcut>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStackedWidget>
#include <QStandardPaths>
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
        // The browser is in the band, its Effects page in front.
        window.m_shell->setSheet(StageShell::Sheet::Browser);
        QVERIFY(window.m_commands->action(QStringLiteral("window.effects")));
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}}}}}}}}}});
        EffectsPanel *panel = window.m_effects;
        // The whole effects catalog is offered (picture and sound), by
        // category and words; transitions and generators are not effects.
        QVERIFY(panel->m_items.size() > 100);
        QVERIFY(panel->m_items.contains(QStringLiteral("light.edgeglow")));
        QVERIFY(panel->m_items.contains(QStringLiteral("audio.limiter")));
        QVERIFY(!panel->m_items.contains(QStringLiteral("push")));
        QVERIFY(!panel->m_items.contains(QStringLiteral("title")));
        panel->m_search->setText(QStringLiteral("neon"));
        QVERIFY(!panel->m_items.value(QStringLiteral("light.edgeglow"))->isHidden());
        QVERIFY(panel->m_items.value(QStringLiteral("channel.invert"))->isHidden());
        // With nothing selected the cards show the program's clip under the playhead.
        QCOMPARE(panel->previewClip(), QStringLiteral("c1"));
        window.selectClip(QStringLiteral("c1"));
        QCOMPARE(panel->m_clip, QStringLiteral("c1"));
        QVERIFY(panel->m_picture);
        // The cards shown ask for previews of themselves on the clip, at
        // twice the thumb's width.
        panel->requestPreviews();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(window.m_lastCommand.value("width").toInt(), EffectsPanel::kPreviewWidth);
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("card"));
        QVERIFY(window.m_lastCommand.value("effects").toArray().contains(QJsonValue("light.edgeglow")));
        QImage red(16, 9, QImage::Format_RGBA8888);
        red.fill(Qt::red);
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", "c1"}, {"fx", "light.edgeglow"}, {"key", "card"},
                                       {"width", 16}, {"height", 9},
                                       {"rgba", QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(red.constBits()), 16 * 9 * 4).toBase64())}});
        const QImage card = panel->m_items.value(QStringLiteral("light.edgeglow"))->icon().pixmap(EffectsPanel::kCard).toImage();
        QCOMPARE(card.pixelColor(card.width() / 2, card.height() / 2), QColor(Qt::red));
        // A card with its preview is not asked for again.
        QVERIFY(!panel->shownEffects().contains(QStringLiteral("light.edgeglow")));
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

    // Bin thumbs (72 x 38 on black) fit a poster at its own shape, draw a
    // sound's waveform on raised, a checkerboard under media with alpha, and
    // the duration in seconds and frames at the sequence's rate.
    void binTilesFitPostersAtTheirShape() {
        MediaArtwork artwork;
        QImage vertical(79, 140, QImage::Format_RGB32);
        vertical.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("tall"), vertical);
        artwork.peaks.insert(QStringLiteral("snd"), fullScalePeaks(4.0));
        ProjectPanel bin(&artwork);
        bin.setFps(25);
        bin.setSources(QJsonArray{
            QJsonObject{{"id", "tall"}, {"path", "media/tall.mov"}, {"width", 1080}, {"height", 1920}, {"duration", 9.8}},
            QJsonObject{{"id", "snd"}, {"path", "media/voice.wav"}, {"duration", 4}, {"hasAudio", true}},
            QJsonObject{{"id", "logo"}, {"path", "media/logo.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 3.44}},
            QJsonObject{{"id", "hiss"}, {"path", "media/steam.wav"}, {"duration", 4}, {"hasAudio", true}}});
        bin.setMediaInfo(QJsonObject{{"logo", QJsonObject{{"codec", "prores"}, {"pixFmt", "yuva444p12le"}}}});
        const auto tile = [&bin](int row) { return bin.m_list->item(row)->icon().pixmap(ProjectPanel::kThumbnail).toImage(); };
        const QImage tall = tile(0);
        QCOMPARE(tall.size() / tall.devicePixelRatio(), ProjectPanel::kThumbnail);
        const QPoint centre(36, 14);
        QCOMPARE(tall.pixelColor(centre * tall.devicePixelRatio()), QColor("#e87342"));
        // 9:16 in a 38 pt tall thumb is 21 wide: black either side.
        QCOMPARE(tall.pixelColor(QPoint(centre.x() - 14, centre.y()) * tall.devicePixelRatio()), QColor(Qt::black));
        QCOMPARE(tall.pixelColor(QPoint(centre.x() + 14, centre.y()) * tall.devicePixelRatio()), QColor(Qt::black));
        QCOMPARE(tall.pixelColor(QPoint(centre.x() - 8, centre.y()) * tall.devicePixelRatio()), QColor("#e87342"));
        // The duration badge at right 4, bottom 4, frames at 25p.
        QCOMPARE(bin.m_list->item(0)->data(Qt::UserRole + 1).toString(), QStringLiteral("9s20f"));
        const QColor badge = tall.pixelColor(QPoint(66, 31) * tall.devicePixelRatio());
        QVERIFY2(badge.red() < 0x60 || badge == QColor(Qt::white), qPrintable(badge.name()));
        const QImage sound = tile(1);
        QVERIFY(isWaveInk(sound.pixelColor(QPoint(20, 19) * sound.devicePixelRatio())));
        // Sound whose peaks have not come yet: raised, with the audio symbol.
        const QImage hiss = tile(3);
        QCOMPARE(hiss.pixelColor(QPoint(2, 2) * hiss.devicePixelRatio()).rgb(), tokens::raised);
        // Alpha media: the 8 pt checkerboard (no picture decoded yet).
        const QImage logo = tile(2);
        QCOMPARE(logo.pixelColor(QPoint(2, 2) * logo.devicePixelRatio()).rgb(), qRgb(0x26, 0x26, 0x26));
        QCOMPARE(logo.pixelColor(QPoint(10, 2) * logo.devicePixelRatio()).rgb(), qRgb(0x30, 0x30, 0x30));
        QVERIFY(ProjectPanel::pixelFormatHasAlpha(QStringLiteral("rgba")));
        QVERIFY(!ProjectPanel::pixelFormatHasAlpha(QStringLiteral("yuv420p")));
        QCOMPARE(ProjectPanel::badgeText(30.0, 25), QStringLiteral("30s00f"));
        QCOMPARE(ProjectPanel::badgeText(6.48, 25), QStringLiteral("6s12f"));
        QCOMPARE(ProjectPanel::badgeText(75.5, 25), QStringLiteral("1m15s"));
        QCOMPARE(ProjectPanel::badgeText(3725, 24), QStringLiteral("1h02m"));
    }

    // The bin at its 272 (271 inside its seam): a 28 strip, the grid 192 on
    // the well, 3 columns of 72, 12 apart, padded 8 16; items 56 tall with
    // rows 8 apart; Roles in the rest.
    void binGridIsThreeColumnsOf72At272() {
        ProjectPanel bin(nullptr);
        QJsonArray sources;
        for (int index = 0; index < 12; ++index)
            sources.append(QJsonObject{{"id", QStringLiteral("m%1").arg(index)}, {"path", QStringLiteral("media/clip%1.mov").arg(index)},
                                       {"width", 1920}, {"height", 1080}, {"duration", 4}});
        bin.setSources(sources);
        QCOMPARE(bin.mediaCount(), 12);
        bin.resize(tokens::layout::bin - 1, tokens::layout::band - 1);
        bin.show();
        QVERIFY(QTest::qWaitForWindowExposed(&bin));
        QCoreApplication::processEvents();
        QCOMPARE(bin.m_pages->height(), ProjectPanel::kGridHeight);
        QCOMPARE(bin.m_pages->y(), tokens::layout::strip);
        const auto thumb = [&bin](int row) {
            const QRect cell = bin.m_list->visualItemRect(bin.m_list->item(row));
            return QRect(bin.m_list->viewport()->mapTo(&bin, cell.topLeft()), ProjectPanel::kThumbnail);
        };
        QCOMPARE(thumb(0).topLeft(), QPoint(ProjectPanel::kSidePadding, tokens::layout::strip + ProjectPanel::kTopPadding));
        QCOMPARE(thumb(1).left(), ProjectPanel::kSidePadding + 72 + 12);
        QCOMPARE(thumb(2).left(), ProjectPanel::kSidePadding + 2 * (72 + 12));
        QCOMPARE(thumb(3).left(), ProjectPanel::kSidePadding);
        QCOMPARE(thumb(3).top() - thumb(0).top(), ProjectPanel::kItemHeight + ProjectPanel::kRowGap);
        // The third column ends inside the bin, clear of its edge.
        QVERIFY(thumb(2).right() < bin.width() - 1);
        // The list view keeps the same media, 24 pt rows.
        bin.setListView(true);
        QCoreApplication::processEvents();
        QCOMPARE(bin.m_list->visualItemRect(bin.m_list->item(1)).top() - bin.m_list->visualItemRect(bin.m_list->item(0)).top(),
                 ProjectPanel::kListRow);
        QCOMPARE(bin.m_viewButton->iconName(), QStringLiteral("g-grid"));
        QCOMPARE(ui::tipName(bin.m_viewButton), QStringLiteral("Grid view"));
        bin.setListView(false);
        QCOMPARE(ui::tipName(bin.m_viewButton), QStringLiteral("List view"));
        // The keys are the registry's (bin.search, bin.toggleView): none
        // on a bin of its own, the commands' once it has them.
        QVERIFY(ui::tipShortcut(bin.m_viewButton).isEmpty());
        QAction search(QStringLiteral("Find in Bin"));
        search.setShortcut(QKeySequence(Qt::CTRL | Qt::Key_F));
        QAction view(QStringLiteral("Switch Bin View"));
        view.setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_2));
        bin.setCommandActions(&search, &view);
        QCOMPARE(ui::tipShortcut(bin.m_viewButton), view.shortcut().toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(bin.m_searchButton), search.shortcut().toString(QKeySequence::NativeText));
        bin.setCommandActions(nullptr, nullptr);
        QVERIFY(!bin.actions().contains(&view));
        // Search shows its field and narrows the media by name.
        bin.focusSearch();
        QVERIFY(bin.m_searchRow->isVisible());
        bin.m_search->setText(QStringLiteral("clip1"));
        int shown = 0;
        for (int row = 0; row < bin.m_list->count(); ++row) shown += bin.m_list->item(row)->isHidden() ? 0 : 1;
        QCOMPARE(shown, 3); // clip1, clip10, clip11
        QTest::keyClick(bin.m_search, Qt::Key_Escape);
        QVERIFY(!bin.m_searchRow->isVisible());
        QVERIFY(bin.m_search->text().isEmpty());
    }

    // The Index tab: the chosen media's shots (its cuts as ranges), silences
    // and loudest moments from media_index; a row shows the media in the
    // Source viewer there.
    void indexRowsComeFromTheMediaIndex() {
        QTemporaryDir project;
        MainWindow window{project.path(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 10}, {"timeline", QJsonObject{
            {"fps", 25}, {"tracks", QJsonArray{}}, {"sources", QJsonArray{
                QJsonObject{{"id", "vo"}, {"path", "media/VO_take3.wav"}, {"duration", 10.4}, {"hasAudio", true}, {"rate", "25"}, {"fps", 25}},
                QJsonObject{{"id", "b3"}, {"path", "media/B003_C011.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 9.8}, {"fps", 25}}}}}}});
        ProjectPanel *bin = window.m_projectPanel;
        QSignalSpy wanted(bin, &ProjectPanel::indexWanted);
        // Nothing chosen: the Index says how to get one.
        bin->setTab(ProjectPanel::Tab::Index);
        QVERIFY(bin->m_indexEmpty->text().contains(QStringLiteral("Choose media")));
        QCOMPARE(wanted.size(), 0);
        bin->setCurrentMedia(QStringLiteral("vo"));
        QCOMPARE(wanted.size(), 1);
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_index"}, {"media", "vo"}}));
        QVERIFY(bin->m_indexEmpty->text().startsWith(QStringLiteral("Indexing VO_take3")));
        // Asked once.
        bin->setTab(ProjectPanel::Tab::Media);
        bin->setTab(ProjectPanel::Tab::Index);
        QCOMPARE(wanted.size(), 1);
        window.handleEvent(QJsonObject{{"type", "media_index"}, {"media", "vo"}, {"cuts", QJsonArray{2.0, 5.48}},
                                       {"ranges", QJsonArray{QJsonObject{{"start", 0.0}, {"end", 2.0}}, QJsonObject{{"start", 2.0}, {"end", 5.48}},
                                                             QJsonObject{{"start", 5.48}, {"end", 10.4}}}},
                                       {"silences", QJsonArray{QJsonArray{6.0, 6.8}}}, {"loudest", QJsonArray{3.2}}});
        QStringList rows;
        for (int row = 0; row < bin->m_index->count(); ++row) {
            const QListWidgetItem *item = bin->m_index->item(row);
            rows << (item->text() + QLatin1Char('|') + item->data(Qt::UserRole + 3).toString());
        }
        QCOMPARE(rows, (QStringList{"3 shots|", "Shot 1|00:00:00:00–00:00:02:00", "Shot 2|00:00:02:00–00:00:05:12",
                                    "Shot 3|00:00:05:12–00:00:10:10", "1 silence|", "Silence 1|00:00:06:00–00:00:06:20",
                                    "Loudest moments|", "Loudest 1|00:00:03:05"}));
        QVERIFY(bin->m_index->isVisibleTo(bin) && !bin->m_indexEmpty->isVisibleTo(bin));
        // Shot 2 shows the media in the Source viewer at its start.
        emit bin->m_index->itemClicked(bin->m_index->item(2));
        QCOMPARE(window.m_sourceMonitor->mediaId(), QStringLiteral("vo"));
        QCOMPARE(window.m_sourceMonitor->position(), 2.0);
        // Without ranges, the shots are the spans between the cuts; a
        // refusal is shown in place of the rows.
        bin->setMediaIndex(QJsonObject{{"media", "vo"}, {"cuts", QJsonArray{4.0}}});
        QCOMPARE(bin->m_index->item(0)->text(), QStringLiteral("2 shots"));
        QCOMPARE(bin->m_index->item(2)->data(Qt::UserRole + 3).toString(), QStringLiteral("00:00:04:00–00:00:10:10"));
        bin->setCurrentMedia(QStringLiteral("b3"));
        window.handleEvent(QJsonObject{{"type", "error"}, {"command", "media_index"}, {"media", "b3"}, {"message", "B003_C011.mov is offline"}});
        QCOMPARE(bin->m_indexEmpty->text(), QStringLiteral("B003_C011.mov is offline"));
        QVERIFY(bin->m_index->isHidden());
    }

    // Roles: the program's tracks grouped by role, tracks without one
    // under their kind; a role's box switches its tracks with one edit, and
    // Edit roles… sends set-track role for each track changed.
    void rolesGroupTracksAndSwitchThem() {
        const auto track = [](const char *id, const char *type, const char *role, bool assigned, bool enabled = true) {
            QJsonObject value{{"id", id}, {"type", type}, {"roleAssigned", assigned}, {"enabled", enabled}, {"clips", QJsonArray{}}};
            if (role) value.insert("role", role);
            return value;
        };
        const QJsonArray tracks{track("V1", "video", "video", false), track("V2", "video", "video", false),
                                track("V3", "video", "titles", true), track("V4", "video", "titles", true, false),
                                track("V5", "video", "Titles", true), track("A1", "audio", "dialogue", false),
                                track("A2", "audio", "music", true), track("A3", "audio", "effects", true),
                                track("A4", "audio", "dialogue", true), track("S1", "subtitle", nullptr, false)};
        const QList<ProjectPanel::RoleGroup> groups = ProjectPanel::roleGroups(tracks);
        QStringList seen;
        for (const ProjectPanel::RoleGroup &group : groups) seen << QStringLiteral("%1 %2 %3").arg(group.name, group.tracks.join(QLatin1Char(' ')), group.enabled ? "on" : "off");
        QCOMPARE(seen, (QStringList{"Video V1 V2 on", "Titles V3 V4 V5 off", "Dialogue A1 A4 on", "Music A2 on", "Effects A3 on",
                                    "Captions S1 on"}));

        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 9}, {"duration", 30}, {"timeline", QJsonObject{{"fps", 25}, {"tracks", tracks}}}});
        ProjectPanel *bin = window.m_projectPanel;
        QCOMPARE(bin->m_roleChecks.size(), 6);
        ui::CheckBox *titles = bin->findChild<ui::CheckBox *>(QStringLiteral("binRoleCheck_titles"));
        QVERIFY(titles);
        QVERIFY(!titles->isChecked());
        titles->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        QCOMPARE(window.m_lastCommand.value("revision").toInt(), 9);
        QCOMPARE(window.m_lastCommand.value("ops").toArray(), BinWiring::enableOps({"V3", "V4", "V5"}, true));
        // The box flipped before kerfd answered; a snapshot with the tracks
        // as they were (the edit refused) puts it back.
        QVERIFY(titles->isChecked());
        window.applySnapshot(QJsonObject{{"revision", 10}, {"duration", 30}, {"timeline", QJsonObject{{"fps", 25}, {"tracks", tracks}}}});
        QCOMPARE(bin->findChild<ui::CheckBox *>(QStringLiteral("binRoleCheck_titles")), titles);
        QVERIFY(!titles->isChecked());
        QCOMPARE(BinWiring::enableOps({"A2"}, false).first().toObject(),
                 (QJsonObject{{"op", "set-track"}, {"track", "A2"}, {"enabled", false}}));
        // Edit roles…: A2 becomes Effects, A4 goes back to its kind's own.
        QCOMPARE(BinWiring::roleChoices(tracks).mid(0, 5), (QStringList{"Video", "Titles", "Dialogue", "Music", "Effects"}));
        QTimer::singleShot(0, &window, [&window] {
            auto *dialog = window.findChild<QDialog *>(QStringLiteral("editRolesDialog"));
            QVERIFY(dialog);
            QTimer::singleShot(2000, dialog, &QDialog::reject);
            auto *music = dialog->findChild<QComboBox *>(QStringLiteral("roleFor_A2"));
            auto *voice = dialog->findChild<QComboBox *>(QStringLiteral("roleFor_A4"));
            auto *captions = dialog->findChild<QComboBox *>(QStringLiteral("roleFor_S1"));
            QVERIFY(music && voice && captions);
            QCOMPARE(music->currentText(), QStringLiteral("Music"));
            QCOMPARE(voice->currentText(), QStringLiteral("Dialogue"));
            QCOMPARE(captions->currentIndex(), 0);
            QCOMPARE(captions->currentText(), QStringLiteral("Captions (default)"));
            music->setCurrentIndex(music->findText(QStringLiteral("Effects")));
            voice->setCurrentIndex(0);
            dialog->findChild<QDialogButtonBox *>()->button(QDialogButtonBox::Ok)->click();
        });
        window.m_lastCommand = {};
        bin->editRolesButton()->click();
        QStringList ops;
        for (const QJsonValue &op : window.m_lastCommand.value("ops").toArray())
            ops << QString::fromUtf8(QJsonDocument(op.toObject()).toJson(QJsonDocument::Compact));
        ops.sort();
        QCOMPARE(ops, (QStringList{R"({"op":"set-track","role":"","track":"A4"})", R"({"op":"set-track","role":"effects","track":"A2"})"}));
    }

    void presetsPanelListsAppliesAndSavesTheLibrary() {
        QSettings().remove(QStringLiteral("workspace"));
        QTemporaryDir project;
        MainWindow window{project.path(), QString()};
        // Presets is a page of the Effects and titles sheet.
        window.triggerCommand(QStringLiteral("window.presets"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Browser);
        QVERIFY(window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
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
        // 44 pt rows: the name, then clips · slots · texts · date.
        QVERIFY2(panel->m_list->item(0)->text().startsWith(QStringLiteral("Intro\n3 clips · 2 slots · 1 text · ")),
                 qPrintable(panel->m_list->item(0)->text()));
        QVERIFY(panel->m_list->item(0)->toolTip().contains(QStringLiteral("Saved by jun")));
        QCOMPARE(panel->m_count->text(), QStringLiteral("2 items"));
        QCOMPARE(panel->m_list->visualItemRect(panel->m_list->item(0)).height(), PresetsPanel::kRowHeight);
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

    // An unreadable preset's name is in neg, and still reads on the chrome.
    void unreadablePresetsTurnRedInEitherAppearance() {
        PresetsPanel panel;
        panel.setLibrary(QJsonObject{{"presets", QJsonArray{
            QJsonObject{{"path", "presets/old.kerfpreset"}, {"name", "old"}, {"error", "line 3: unknown statement"}},
            QJsonObject{{"path", "presets/intro.kerfpreset"}, {"name", "Intro"}, {"slots", QJsonArray{}}}}}}, QDir::tempPath());
        QListWidgetItem *unreadable = panel.m_list->item(0);
        QListWidgetItem *readable = panel.m_list->item(1);
        QCOMPARE(unreadable->foreground().color(), tokens::color(tokens::neg));
        QVERIFY(contrastRatio(unreadable->foreground().color(), tokens::color(tokens::chrome)) >= 4.5);
        QVERIFY(!readable->data(Qt::ForegroundRole).isValid());
        QCOMPARE(PresetsPanel::metaLine(QJsonObject{{"error", "bad"}}), QStringLiteral("Cannot be read: bad"));
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
    // The category menu counts the catalog: every picture effect, each of
    // its categories in catalog order, then the sound effects; the count
    // follows the category and the search.
    void browserCategoriesCountTheCatalog() {
        const QJsonArray catalog{
            QJsonObject{{"kind", "effect"}, {"domain", "video"}, {"id", "color.board"}, {"name", "Color board"}, {"category", "Color"}},
            QJsonObject{{"kind", "effect"}, {"domain", "video"}, {"id", "color.bw"}, {"name", "Black & white"}, {"category", "Color"}},
            QJsonObject{{"kind", "effect"}, {"domain", "video"}, {"id", "blur.gaussian"}, {"name", "Gaussian blur"}, {"category", "Blur and sharpen"}},
            QJsonObject{{"kind", "effect"}, {"domain", "audio"}, {"id", "audio.eq"}, {"name", "Channel EQ"}, {"category", "Audio"}},
            QJsonObject{{"kind", "effect"}, {"domain", "audio"}, {"id", "audio.limiter"}, {"name", "Limiter"}, {"category", "Audio"}},
            QJsonObject{{"kind", "transition"}, {"domain", "video"}, {"id", "dissolve"}, {"name", "Cross dissolve"}, {"category", "Transition"}},
            QJsonObject{{"kind", "generator"}, {"domain", "video"}, {"id", "title"}, {"name", "A line of text. Position it with x and y"},
                        {"category", "Generate"}}};
        EffectsPanel effects;
        effects.setCatalog(catalog);
        QStringList rows;
        for (const EffectsPanel::Category &category : effects.categories())
            rows << QStringLiteral("%1%2 %3").arg(category.separatorBefore ? "— " : "", category.name).arg(category.count);
        QCOMPARE(rows, (QStringList{"All video effects 3", "Color 2", "Blur and sharpen 1", "— All audio effects 2"}));
        QCOMPARE(effects.category(), QStringLiteral("video"));
        QCOMPARE(effects.m_count->text(), QStringLiteral("3 items"));
        QCOMPARE(effects.m_categoryButton->text(), QStringLiteral("All video effects"));
        QVERIFY(effects.m_categoryButton->hasChevron());
        // The ghost button fits its text: it is sized once before the catalog names it.
        QCOMPARE(effects.m_categoryButton->size(), effects.m_categoryButton->sizeHint());
        effects.setCategory(QStringLiteral("video:Color"));
        QCOMPARE(effects.shownCount(), 2);
        QCOMPARE(effects.m_categoryButton->text(), QStringLiteral("Color"));
        QCOMPARE(effects.m_categoryButton->size(), effects.m_categoryButton->sizeHint());
        effects.setCategory(QStringLiteral("audio"));
        QCOMPARE(effects.m_count->text(), QStringLiteral("2 items"));
        QVERIFY(effects.m_items.value(QStringLiteral("blur.gaussian"))->isHidden());
        // Sound effects have no picture to preview.
        effects.setClip(QStringLiteral("c1"), true);
        QVERIFY(effects.shownEffects().isEmpty());
        // The menu: a checked row per category, its count on the right.
        std::unique_ptr<QMenu> menu(effects.categoryMenu());
        QStringList items;
        for (QAction *action : menu->actions())
            items << (action->isSeparator() ? QStringLiteral("—") : action->text() + (action->isChecked() ? QStringLiteral(" ✓") : QString()));
        QCOMPARE(items, (QStringList{"All video effects", "Color", "Blur and sharpen", "—", "All audio effects ✓"}));
        menu->actions().at(1)->trigger();
        QCOMPARE(effects.category(), QStringLiteral("video:Color"));
        // Search narrows within the category; nothing found says so.
        effects.focusSearch();
        effects.m_search->setText(QStringLiteral("zebra"));
        QCOMPARE(effects.m_count->text(), QStringLiteral("0 items"));
        QVERIFY(effects.m_hint->text().contains(QStringLiteral("zebra")));
        // No catalog (no engine): an empty state, not sample cards.
        EffectsPanel none;
        none.setCatalog({});
        QVERIFY(none.m_hint->text().contains(QStringLiteral("catalog is not available")));
        QVERIFY(!none.m_categoryButton->isEnabled());
        // The real catalog: its generators are the Titles, and its sound
        // effects count under All audio effects.
        char *all = kerf_fx_catalog_all();
        QVERIFY(all);
        const QJsonArray real = QJsonDocument::fromJson(QByteArray(all)).array();
        kerf_bridge_string_free(all);
        EffectsPanel catalogEffects;
        catalogEffects.setCatalog(real);
        QVERIFY(catalogEffects.categories().constLast().key == QStringLiteral("audio"));
        QVERIFY(catalogEffects.categories().constLast().count >= 3);
        EffectsPanel catalogTitles(nullptr, EffectsPanel::Mode::Titles);
        catalogTitles.setCatalog(real);
        QVERIFY(catalogTitles.m_items.contains(QStringLiteral("title")));
        QVERIFY(catalogTitles.m_items.contains(QStringLiteral("solid")));
        QCOMPARE(catalogTitles.m_items.value(QStringLiteral("title"))->text(), QStringLiteral("A line of text"));
        QCOMPARE(catalogTitles.categories().constFirst().name, QStringLiteral("All titles"));
    }

    // Hovering a thumb scrubs the effect's main parameter by the pointer's
    // x: effect_previews with {fx, params}, at most once per 120 ms (the
    // latest position going when the interval is up); its answer shows on
    // the card, and leaving puts the resting thumb back.
    void hoverScrubSendsEffectPreviewsWithParams() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        EffectsPanel *panel = window.m_effects;
        qint64 clock = 1000;
        panel->setClock([&clock] { return clock; });
        const QJsonObject radius = EffectsPanel::mainParam(panel->m_fx.value(QStringLiteral("blur.gaussian")));
        QCOMPARE(radius.value("name").toString(), QStringLiteral("radius"));
        QCOMPARE(radius.value("max").toDouble(), 500.0);
        const auto scrubbed = [&window] {
            const QJsonObject entry = window.m_lastCommand.value("effects").toArray().first().toObject();
            return entry.value("params").toObject().value("radius").toDouble();
        };
        panel->scrubAt(QStringLiteral("blur.gaussian"), 0.5);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("scrub"));
        QCOMPARE(window.m_lastCommand.value("effects").toArray().first().toObject().value("fx").toString(), QStringLiteral("blur.gaussian"));
        QCOMPARE(window.m_lastCommand.value("effects").toArray().first().toObject().value("key").toString(), QStringLiteral("scrub"));
        QCOMPARE(scrubbed(), 250.0);
        // 50 ms later: held back, the latest position waiting.
        window.m_lastCommand = {};
        clock += 50;
        panel->scrubAt(QStringLiteral("blur.gaussian"), 0.2);
        panel->scrubAt(QStringLiteral("blur.gaussian"), 0.25);
        QCOMPARE(window.m_lastCommand, QJsonObject{});
        QVERIFY(panel->m_scrubTimer->isActive());
        QCOMPARE(panel->m_scrubTimer->interval(), 70);
        clock += 70;
        panel->m_scrubTimer->stop();
        panel->sendScrub();
        QCOMPARE(scrubbed(), 125.0);
        // The resting thumb, then the scrub's frame over it; leaving puts it back.
        const auto rgba = [](const QColor &color) {
            QImage image(16, 9, QImage::Format_RGBA8888);
            image.fill(color);
            return QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(image.constBits()), 16 * 9 * 4).toBase64());
        };
        const auto cardCentre = [panel] {
            const QImage card = panel->m_items.value(QStringLiteral("blur.gaussian"))->icon().pixmap(EffectsPanel::kCard).toImage();
            return card.pixelColor(card.width() / 2, card.height() / 2);
        };
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", "c1"}, {"fx", "blur.gaussian"}, {"key", "card"},
                                       {"width", 16}, {"height", 9}, {"rgba", rgba(Qt::blue)}});
        // While the pointer is on the card the resting thumb waits.
        QVERIFY(cardCentre() != QColor(Qt::blue));
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", "c1"}, {"fx", "blur.gaussian"}, {"key", "scrub"},
                                       {"width", 16}, {"height", 9}, {"rgba", rgba(Qt::green)}});
        QCOMPARE(cardCentre(), QColor(Qt::green));
        QCOMPARE(panel->m_base.value(QStringLiteral("blur.gaussian")).pixelColor(0, 0), QColor(Qt::blue));
        panel->endScrub();
        QCOMPARE(cardCentre(), QColor(Qt::blue));
        QVERIFY(panel->scrubbing().isEmpty());
        // An effect with no ranged number is not scrubbed.
        window.m_lastCommand = {};
        clock += 500;
        panel->scrubAt(QStringLiteral("channel.invert"), 0.5);
        if (EffectsPanel::mainParam(panel->m_fx.value(QStringLiteral("channel.invert"))).isEmpty()) QCOMPARE(window.m_lastCommand, QJsonObject{});
    }

    // effect_preview answers another region asked for (a proposal card's
    // "before:"/"after:" frames) carry that request's params: no card takes
    // them, and the page in front asks again for what it was cut short of.
    void foreignPreviewKeysLeaveCardsAlone() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        window.m_browserPanel->setPage(BrowserPanel::Page::Effects);
        EffectsPanel *panel = window.m_effects;
        EffectsPanel *titles = window.m_binWiring->titles();
        QCOMPARE(panel->previewClip(), QStringLiteral("c1"));
        const QString fx = QStringLiteral("blur.gaussian");
        QVERIFY(panel->m_items.contains(fx));
        const auto rgba = [](const QColor &color) {
            QImage image(16, 9, QImage::Format_RGBA8888);
            image.fill(color);
            return QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(image.constBits()), 16 * 9 * 4).toBase64());
        };
        const auto preview = [&rgba](const QString &clip, const QString &id, const QString &key, const QColor &color) {
            QJsonObject event{{"type", "effect_preview"}, {"clip", clip}, {"fx", id}, {"width", 16}, {"height", 9}, {"rgba", rgba(color)}};
            if (!key.isNull()) event.insert("key", key);
            return event;
        };
        const auto cardCentre = [panel, &fx] {
            const QImage card = panel->m_items.value(fx)->icon().pixmap(EffectsPanel::kCard).toImage();
            return card.pixelColor(card.width() / 2, card.height() / 2);
        };
        const QColor resting = cardCentre();
        QVERIFY(panel->shownEffects().contains(fx));

        // A proposal's frames for the shown clip and card: dropped, and the
        // card is still one to ask for.
        panel->m_previewTimer->stop();
        window.handleEvent(preview("c1", fx, "after:p1", Qt::red));
        window.handleEvent(preview("c1", fx, "before:p1", Qt::red));
        QCOMPARE(cardCentre(), resting);
        QVERIFY(!panel->m_base.contains(fx));
        QVERIFY(panel->shownEffects().contains(fx));
        // That other request stopped the page's own: it asks again soon.
        QVERIFY(panel->m_previewTimer->isActive());
        // The Titles page took nothing either.
        QVERIFY(titles->m_base.isEmpty());

        // The other page's key is not this one's either.
        panel->setPreview(QStringLiteral("c1"), fx, QString::fromLatin1(EffectsPanel::kTitleKey), QImage(16, 9, QImage::Format_RGB32));
        QVERIFY(!panel->m_base.contains(fx));

        // The page's own answer is a thumb and asks for nothing more.
        panel->m_previewTimer->stop();
        window.handleEvent(preview("c1", fx, "card", Qt::blue));
        QCOMPARE(cardCentre(), QColor(Qt::blue));
        QCOMPARE(panel->m_base.value(fx).pixelColor(0, 0), QColor(Qt::blue));
        QVERIFY(!panel->shownEffects().contains(fx));
        QVERIFY(!panel->m_previewTimer->isActive());
        // A later proposal frame leaves it as it is.
        window.handleEvent(preview("c1", fx, "after:p1", Qt::red));
        QCOMPARE(cardCentre(), QColor(Qt::blue));
        QCOMPARE(panel->m_base.value(fx).pixelColor(0, 0), QColor(Qt::blue));
        // An unkeyed answer (an older kerfd) is still a resting thumb.
        window.handleEvent(preview("c1", fx, QString(), Qt::green));
        QCOMPARE(panel->m_base.value(fx).pixelColor(0, 0), QColor(Qt::green));
    }

    // Titles: the catalog's generators, each previewed alone on black
    // (effect_previews with clip "" and key title); adding one is a
    // generator insert at the playhead (titlesAddAGeneratorClipThroughKerfd).
    void titlesListGeneratorsWithTheirPreviews() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        BrowserPanel *browser = window.m_browserPanel;
        EffectsPanel *titles = window.m_binWiring->titles();
        QVERIFY(titles);
        QCOMPARE(browser->titles(), titles);
        QVERIFY(browser->pageButton(BrowserPanel::Page::Titles)->isEnabled());
        browser->pageButton(BrowserPanel::Page::Titles)->click();
        QCOMPARE(browser->page(), BrowserPanel::Page::Titles);
        QCOMPARE(browser->currentPage(), titles);
        // Without a bridge the page does not ask on its own; asked, it
        // wants every generator with a picture.
        QVERIFY(window.m_lastCommand.value("type").toString() != QStringLiteral("effect_previews"));
        titles->requestPreviews();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QString());
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("title"));
        const QJsonArray asked = window.m_lastCommand.value("effects").toArray();
        QVERIFY(asked.contains(QJsonValue("title")) && asked.contains(QJsonValue("solid")));
        // The adjustment layer has no picture.
        QVERIFY(!asked.contains(QJsonValue("adjustment")));
        QImage white(16, 9, QImage::Format_RGBA8888);
        white.fill(Qt::white);
        window.handleEvent(QJsonObject{{"type", "effect_preview"}, {"clip", ""}, {"fx", "title"}, {"key", "title"}, {"width", 16}, {"height", 9},
                                       {"rgba", QString::fromLatin1(QByteArray(reinterpret_cast<const char *>(white.constBits()), 16 * 9 * 4).toBase64())}});
        const QImage card = titles->m_items.value(QStringLiteral("title"))->icon().pixmap(EffectsPanel::kCard).toImage();
        QCOMPARE(card.pixelColor(card.width() / 2, card.height() / 2), QColor(Qt::white));
        // The Effects page did not take the generator's picture.
        QVERIFY(!window.m_effects->m_items.contains(QStringLiteral("title")));
        // Adding is available: the cards say how, and drag as generators.
        QVERIFY(titles->addAvailable());
        QVERIFY(titles->m_items.value(QStringLiteral("title"))->toolTip().contains(QStringLiteral("drag to a video track")));
        QCOMPARE(titles->m_cards->model()->mimeTypes(), QStringList{QString::fromLatin1(EffectsPanel::kGeneratorMime)});
        QCOMPARE(window.m_effects->m_cards->model()->mimeTypes(), QStringList{QString::fromLatin1(EffectsPanel::kFxMime)});
        // Search follows the page in front.
        browser->focusSearch();
        QVERIFY(titles->m_searchRow->isVisibleTo(titles));
    }

    // A program of 25 fps with the tracks given, as kerfd sends it.
    static QJsonObject titleSnapshot(int revision, const QJsonArray &tracks) {
        return QJsonObject{{"revision", revision}, {"duration", 10},
                           {"timeline", QJsonObject{{"fps", 25}, {"rate", "25"}, {"sequence", "main"}, {"tracks", tracks}}}};
    }
    static QJsonObject videoTrack(const QString &id, const QJsonArray &clips = {}) {
        return QJsonObject{{"id", id}, {"type", "video"}, {"clips", clips}};
    }

    // Titles ▸ Add (double-click, Return or a drag to a video track): one
    // generator insert at the playhead, 5 s at the sequence's rate, over
    // what is there, on the target video track above V1 (else the first
    // above it, else a new track); kerfd's answer shows on the page.
    void titlesAddAGeneratorClipThroughKerfd() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonArray clips{QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 8.0}}};
        window.applySnapshot(titleSnapshot(3, QJsonArray{videoTrack("V1", clips), videoTrack("V2"), videoTrack("V3"),
                                                          QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{}}}}));
        window.queueSeek(2.0);
        BinWiring *wiring = window.m_binWiring.get();
        EffectsPanel *titles = wiring->titles();
        QListWidgetItem *title = titles->m_items.value(QStringLiteral("title"));
        QVERIFY(title);
        const auto lastOps = [&window] { return window.m_lastCommand.value("ops").toArray(); };
        const QJsonObject expected{{"op", "insert"},
                                   {"generator", QJsonObject{{"fx", "title"}, {"params", QJsonObject{{"text", BinWiring::defaultTitleText()}}}}},
                                   {"at", 50},
                                   {"frames", 125},
                                   {"tracks", QJsonArray{"V2"}},
                                   {"mode", "overwrite"}};

        // Double-click: on V2, the first video track above V1.
        window.m_lastCommand = {};
        emit titles->m_cards->itemActivated(title);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        QCOMPARE(lastOps(), QJsonArray{expected});
        // No bridge took it here: the page says so, in neg.
        QVERIFY(titles->statusIsError());
        QVERIFY(titles->status().contains(QStringLiteral("not connected")));
        QVERIFY(titles->m_status->isVisibleTo(titles));

        // A targeted video track above V1 is where it goes.
        window.m_timeline->setTargetsEditable(true);
        window.m_timeline->setTargetTracks({QStringLiteral("V3"), QStringLiteral("A1")});
        QVERIFY(wiring->addGenerator(QStringLiteral("title")) == false);
        QCOMPARE(lastOps().first().toObject().value("tracks").toArray(), QJsonArray{"V3"});
        // A solid has no text of its own.
        wiring->addGenerator(QStringLiteral("solid"));
        QCOMPARE(lastOps().first().toObject().value("generator").toObject(), (QJsonObject{{"fx", "solid"}}));

        // Dropped on V1's row, it goes where Add puts it (V3, the target):
        // an overwrite on V1 would replace the picture.
        int v1 = -1;
        for (int row = 0; row < window.m_timeline->trackRowCount(); ++row)
            if (window.m_timeline->trackRowId(row) == QStringLiteral("V1")) v1 = row;
        QVERIFY(v1 >= 0);
        auto *mime = new QMimeData;
        mime->setData(QString::fromLatin1(EffectsPanel::kGeneratorMime), QByteArray("title"));
        const QPointF at(window.m_timeline->trackHeaderRect(v1).right() + 40, window.m_timeline->trackHeaderRect(v1).center().y());
        QDragEnterEvent enter(at.toPoint(), Qt::CopyAction, mime, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(window.m_timeline, &enter);
        QVERIFY(enter.isAccepted());
        QDropEvent drop(at, Qt::CopyAction, mime, Qt::LeftButton, Qt::NoModifier);
        window.m_lastCommand = {};
        QApplication::sendEvent(window.m_timeline, &drop);
        QCOMPARE(lastOps().first().toObject().value("tracks").toArray(), QJsonArray{"V3"});
        // It lands at the time under the pointer, not at the playhead.
        {
            const double fps = window.m_bus->fps() > 0 ? window.m_bus->fps() : 24.0;
            const double under = window.m_timeline->laneTimeAt(at.x());
            QVERIFY(under >= 0);
            QCOMPARE(lastOps().first().toObject().value("at").toInteger(), qint64(std::llround(under * fps)));
            // Over the track heads there is no time (a drop there goes at
            // the playhead).
            QCOMPARE(window.m_timeline->laneTimeAt(window.m_timeline->trackHeaderRect(v1).center().x()), -1.0);
        }
        // An effect card held over the timeline is the timeline's own.
        auto *effect = new QMimeData;
        effect->setData(QString::fromLatin1(EffectsPanel::kFxMime), QByteArray("blur.gaussian"));
        QDropEvent effectDrop(at, Qt::CopyAction, effect, Qt::LeftButton, Qt::NoModifier);
        window.m_lastCommand = {};
        QApplication::sendEvent(window.m_timeline, &effectDrop);
        QVERIFY(window.m_lastCommand.value("ops").toArray().isEmpty() ||
                window.m_lastCommand.value("ops").toArray().first().toObject().value("op").toString() != QStringLiteral("insert"));
        delete effect;
        delete mime;

        // kerfd's answers, as they come for a sent edit: a refusal stays on
        // the page in kerfd's words; a newer revision holding the clip is a
        // confirmation.
        wiring->m_titlePending = true;
        wiring->m_titleLabel = QStringLiteral("Title on V2 at 00:00:02:00");
        wiring->m_titleRevision = 3;
        wiring->m_titleFx = QStringLiteral("title");
        wiring->m_titleTrack = QStringLiteral("V2");
        wiring->m_titleAt = 50;
        wiring->m_titleFps = 25;
        wiring->m_titleBefore = {QStringLiteral("g0")};
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", "no generator `title`"}});
        QVERIFY(!wiring->titlePending());
        QCOMPARE(titles->status(), QStringLiteral("Title on V2 at 00:00:02:00 was not added: no generator `title`"));
        QVERIFY(titles->statusIsError());
        // Another region's refusal is not the page's.
        wiring->m_titlePending = true;
        window.handleEvent(QJsonObject{{"type", "error"}, {"command", "media_index"}, {"message", "unreadable"}});
        QVERIFY(wiring->titlePending());
        // A newer revision without the clip (an agent's edit, another
        // client's; a title that was there already) is no confirmation.
        const QJsonObject older{{"id", "g0"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 2.0}, {"duration", 5.0}};
        window.handleEvent(QJsonObject{{"type", "state"}, {"snapshot", titleSnapshot(4, QJsonArray{videoTrack("V1", clips), videoTrack("V2", QJsonArray{older})})}});
        QVERIFY(wiring->titlePending());
        QVERIFY(titles->status().isEmpty() || !titles->status().startsWith(QStringLiteral("Added")));
        const QJsonObject added{{"id", "g1"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 2.0}, {"duration", 5.0}};
        window.handleEvent(QJsonObject{{"type", "state"}, {"snapshot", titleSnapshot(5, QJsonArray{videoTrack("V1", clips), videoTrack("V2", QJsonArray{added})})}});
        QVERIFY(!wiring->titlePending());
        QCOMPARE(titles->status(), QStringLiteral("Added Title on V2 at 00:00:02:00"));
        QVERIFY(!titles->statusIsError());
        // No snapshot with the clip before the wait runs out: not added.
        wiring->m_titlePending = true;
        wiring->m_titleTimer->start(1);
        QTRY_VERIFY(!wiring->titlePending());
        QCOMPARE(titles->status(), QStringLiteral("Title on V2 at 00:00:02:00 was not added: Kerf did not answer"));
        QVERIFY(titles->statusIsError());

        // With V1 alone, a video track is added for it first.
        window.applySnapshot(titleSnapshot(6, QJsonArray{videoTrack("V1", clips)}));
        window.m_timeline->setTargetTracks({QStringLiteral("V1")});
        wiring->addGenerator(QStringLiteral("title"));
        QCOMPARE(lastOps().size(), 2);
        QCOMPARE(lastOps().at(0).toObject(), (QJsonObject{{"op", "add-track"}, {"kind", "video"}, {"id", "V2"}}));
        QCOMPARE(lastOps().at(1).toObject().value("tracks").toArray(), QJsonArray{"V2"});

        // Offline nothing goes, and the page says why.
        window.setConnection(QStringLiteral("disconnected"));
        window.m_lastCommand = {};
        QVERIFY(!wiring->addGenerator(QStringLiteral("title")));
        QVERIFY(window.m_lastCommand.isEmpty());
        QVERIFY(titles->status().contains(QStringLiteral("waiting for the project connection")));
    }

    // A subtitle track without a name reads Captions, numbered with the
    // picture tracks (mockup-spec 7: V5 Captions).
    void captionTracksReadAsCaptions() {
        const QJsonObject project{{"tracks", QJsonArray{videoTrack("V1"), QJsonObject{{"id", "S1"}, {"type", "subtitle"}},
                                                        QJsonObject{{"id", "A1"}, {"type", "audio"}}}}};
        const TrackBadge caption = trackBadge(project, QStringLiteral("S1"));
        QCOMPARE(caption.badge, QStringLiteral("V2"));
        QCOMPARE(caption.name, QStringLiteral("Captions"));
        QCOMPARE(trackBadge(project, QStringLiteral("V1")).name, QStringLiteral("Video 1"));
        QCOMPARE(trackBadge(project, QStringLiteral("A1")).name, QStringLiteral("Audio 1"));
        QVERIFY(isCaptionTrack(QJsonObject{{"type", "subtitle"}}));
        QVERIFY(!isCaptionTrack(QJsonObject{{"type", "video"}}));
    }

    // The ops and track choice on their own.
    void titleTracksAndOps() {
        const QJsonObject timeline{{"tracks", QJsonArray{videoTrack("V1"), QJsonObject{{"id", "V2"}, {"type", "video"}, {"locked", true}},
                                                         videoTrack("V3"), QJsonObject{{"id", "A1"}, {"type", "audio"}}}}};
        // A locked track takes nothing: the first unlocked above V1.
        QCOMPARE(BinWiring::titleTrack(timeline, {}), QStringLiteral("V3"));
        QCOMPARE(BinWiring::titleTrack(timeline, {QStringLiteral("V2")}), QStringLiteral("V3"));
        // A drop on an audio track goes where Add would.
        QCOMPARE(BinWiring::titleTrack(timeline, {}, QStringLiteral("A1")), QStringLiteral("V3"));
        // A drop on V1 goes above it too; one on a free track above V1 stays.
        QCOMPARE(BinWiring::titleTrack(timeline, {}, QStringLiteral("V1")), QStringLiteral("V3"));
        QCOMPARE(BinWiring::titleTrack(timeline, {}, QStringLiteral("V3")), QStringLiteral("V3"));
        QCOMPARE(BinWiring::titleTrack(timeline, {QStringLiteral("V3")}, QStringLiteral("V2")), QStringLiteral("V3"));
        // A snapshot shows an add by a new generator clip of it at the frame.
        const QJsonObject held{{"tracks", QJsonArray{videoTrack("V1"), videoTrack("V2", QJsonArray{QJsonObject{
                                   {"id", "g1"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 2.0}, {"duration", 5.0}}})}}};
        QVERIFY(BinWiring::holdsGenerator(held, QStringLiteral("V2"), QStringLiteral("title"), 50, 25));
        QVERIFY(!BinWiring::holdsGenerator(held, QStringLiteral("V2"), QStringLiteral("title"), 50, 25, {QStringLiteral("g1")}));
        QVERIFY(!BinWiring::holdsGenerator(held, QStringLiteral("V2"), QStringLiteral("solid"), 50, 25));
        QVERIFY(!BinWiring::holdsGenerator(held, QStringLiteral("V2"), QStringLiteral("title"), 75, 25));
        QVERIFY(!BinWiring::holdsGenerator(held, QStringLiteral("V1"), QStringLiteral("title"), 50, 25));
        QCOMPARE(BinWiring::generatorClips(held), (QSet<QString>{QStringLiteral("g1")}));
        QCOMPARE(BinWiring::newVideoTrackId(timeline), QStringLiteral("V4"));
        QCOMPARE(BinWiring::newVideoTrackId(QJsonObject{{"tracks", QJsonArray{videoTrack("V1")}}}), QStringLiteral("V2"));
        QCOMPARE(BinWiring::titleTrack(QJsonObject{{"tracks", QJsonArray{videoTrack("V1")}}}, {}), QString());
    }

    // With no picture clip selected, the cards show the clip under the
    // playhead, else the sequence's first picture clip; the placeholder
    // only when there is no picture clip at all.
    void effectsPreviewOnTheProgramsFirstPictureClip() {
        const QJsonArray later{QJsonObject{{"id", "c5"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 5.0}, {"duration", 2.0}},
                               QJsonObject{{"id", "c3"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 3.0}, {"duration", 1.0}}};
        const QJsonObject timeline{{"tracks", QJsonArray{QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                                       QJsonObject{{"id", "a1"}, {"kind", "media"}, {"timelineStart", 0.0}, {"duration", 9.0}}}}},
                                                         videoTrack("V1", later)}}};
        QCOMPARE(BinWiring::previewClipFor(timeline, 5.5), QStringLiteral("c5"));
        QCOMPARE(BinWiring::previewClipFor(timeline, 0.0), QStringLiteral("c3"));
        QCOMPARE(BinWiring::previewClipFor(QJsonObject{{"tracks", QJsonArray{videoTrack("V1")}}}, 0.0), QString());

        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(titleSnapshot(3, QJsonArray{videoTrack("V1", later)}));
        EffectsPanel *effects = window.m_effects;
        QCOMPARE(effects->previewClip(), QStringLiteral("c3"));
        effects->requestPreviews();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("effect_previews"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("c3"));
        window.queueSeek(5.5);
        QCOMPARE(effects->previewClip(), QStringLiteral("c5"));
        window.applySnapshot(titleSnapshot(4, QJsonArray{videoTrack("V1")}));
        QCOMPARE(effects->previewClip(), QString());
    }

    // ⌘F, ⌘⌥2 and ⌘⇧F are the registry's bin.search, bin.toggleView and
    // browser.search (no QShortcut of the panels' own is left), so the
    // keymap lists them and one's own keys reach them.
    void binAndBrowserKeysAreRegistryCommands() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ProjectPanel *bin = window.m_projectPanel;
        BrowserPanel *browser = window.m_browserPanel;
        QVERIFY(bin->findChildren<QShortcut *>().isEmpty());
        QVERIFY(browser->findChildren<QShortcut *>().isEmpty());
        QAction *search = window.m_commands->action(QStringLiteral("bin.search"));
        QAction *view = window.m_commands->action(QStringLiteral("bin.toggleView"));
        QAction *find = window.m_commands->action(QStringLiteral("browser.search"));
        QVERIFY(search && view && find);
        QCOMPARE(search->shortcut(), QKeySequence(Qt::CTRL | Qt::Key_F));
        QCOMPARE(view->shortcut(), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_2));
        QCOMPARE(find->shortcut(), QKeySequence(Qt::CTRL | Qt::SHIFT | Qt::Key_F));
        QVERIFY(bin->actions().contains(search) && bin->actions().contains(view));
        QVERIFY(browser->actions().contains(find));
        // The bin's ⌘F is the bin's while it has the focus (the comments
        // panel keeps its own ⌘F).
        QCOMPARE(search->shortcutContext(), Qt::WidgetWithChildrenShortcut);
        // The strip's tips name the keymap's keys.
        QCOMPARE(ui::tipShortcut(bin->m_searchButton), search->shortcut().toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(bin->m_viewButton), view->shortcut().toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(browser->searchButton()), find->shortcut().toString(QKeySequence::NativeText));
        // They run what the panels' keys ran.
        const bool list = bin->isListView();
        view->trigger();
        QCOMPARE(bin->isListView(), !list);
        search->trigger();
        QVERIFY(bin->m_searchRow->isVisibleTo(bin));
        find->trigger();
        QVERIFY(window.m_effects->m_searchRow->isVisibleTo(window.m_effects));
        // A key of one's own follows into the tips.
        view->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_9));
        QCOMPARE(ui::tipShortcut(bin->m_viewButton), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_9).toString(QKeySequence::NativeText));
        view->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_2));
    }

    // The browser at its 304 (303 inside its seam): 3 columns of 88 x 50
    // thumbs, 8 apart, padded 8 12, rows 12 apart.
    void browserGridIsThreeColumnsOf88At304() {
        EffectsPanel effects;
        QJsonArray catalog;
        for (int index = 0; index < 7; ++index)
            catalog.append(QJsonObject{{"kind", "effect"}, {"domain", "video"}, {"id", QStringLiteral("fx%1").arg(index)},
                                       {"name", QStringLiteral("Effect %1").arg(index)}, {"category", "Color"}});
        effects.setCatalog(catalog);
        effects.resize(tokens::layout::browser - 1, tokens::layout::band - 1 - tokens::layout::strip);
        effects.show();
        QVERIFY(QTest::qWaitForWindowExposed(&effects));
        QCoreApplication::processEvents();
        const auto thumb = [&effects](int row) {
            const QRect cell = effects.m_cards->visualItemRect(effects.m_cards->item(row));
            return QRect(effects.m_cards->viewport()->mapTo(&effects, cell.topLeft()), EffectsPanel::kCard);
        };
        // Under the 24 category row, 4 under the strip.
        QCOMPARE(thumb(0).topLeft(), QPoint(EffectsPanel::kPadX, EffectsPanel::kCategoryRow + 4 + EffectsPanel::kPadY));
        QCOMPARE(thumb(1).left(), EffectsPanel::kPadX + 88 + 8);
        QCOMPARE(thumb(2).left(), EffectsPanel::kPadX + 2 * (88 + 8));
        QCOMPARE(thumb(3).left(), EffectsPanel::kPadX);
        QCOMPARE(thumb(3).top() - thumb(0).top(), 50 + 4 + 16 + 12);
        QVERIFY(thumb(2).right() <= effects.width() - EffectsPanel::kPadX);
        // The category ghost and the count share the row.
        QCOMPARE(effects.m_categoryButton->mapTo(&effects, QPoint()).x(), 4);
        QCOMPARE(effects.m_count->mapTo(&effects, QPoint(effects.m_count->width(), 0)).x(), effects.width() - EffectsPanel::kPadX);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::BinBrowserTest)

#include "bin_browser_test.moc"
