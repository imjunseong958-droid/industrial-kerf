#include "fixtures.h"

#include "chat_panel.h"
#include "editor_theme.h"
#include "main_window.h"
#include "media_artwork.h"
#include "test_footage.h"

#include <QAbstractItemView>
#include <QApplication>
#include <QCoreApplication>
#include <QInputDialog>
#include <QScrollArea>
#include <QScrollBar>
#include <QTest>

#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>

namespace kerf::test {

QImage paintedAt(qreal ratio, const QSize &logical, const std::function<void(QPainter &)> &paint) {
    QImage image(QSize(qCeil(logical.width() * ratio), qCeil(logical.height() * ratio)), QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(ratio);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    paint(painter);
    return image;
}

int lightnessAt(const QImage &image, const QPointF &at) {
    const qreal ratio = image.devicePixelRatio();
    return QColor::fromRgba(image.pixel(int(at.x() * ratio), int(at.y() * ratio))).lightness();
}

// The parts of the fixtures that reach into the editor's private members,
// which only the test classes and this one are friends of (test_friends.h).
struct Access {
    static void showAssistant(MainWindow &window) {
        window.setChatOpen(true);
        QCoreApplication::processEvents();
    }

    static QJsonArray takeOps(MainWindow &window) {
        const QJsonObject command = window.m_lastCommand;
        window.m_lastCommand = {};
        return command.value("type").toString() == QStringLiteral("edit") ? command.value("ops").toArray() : QJsonArray{};
    }
};

void showAssistant(MainWindow &window) { Access::showAssistant(window); }

QJsonObject catalogProperties() {
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

QJsonObject catalogGenerators() {
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

QJsonArray takeOps(MainWindow &window) { return Access::takeOps(window); }

void mouse(QWidget *widget, QEvent::Type type, const QPointF &at) {
    const Qt::MouseButton button = type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton;
    const Qt::MouseButtons buttons = type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::MouseButtons(Qt::LeftButton);
    QMouseEvent event(type, at, widget->mapToGlobal(at), button, buttons, Qt::NoModifier);
    QCoreApplication::sendEvent(widget, &event);
}

void key(QWidget *widget, int code) {
    QKeyEvent event(QEvent::KeyPress, code, Qt::NoModifier);
    QCoreApplication::sendEvent(widget, &event);
}

bool near(const QJsonValue &values, std::initializer_list<double> expected) {
    const QJsonArray array = values.toArray();
    if (array.size() != static_cast<int>(expected.size())) return false;
    int index = 0;
    for (double value : expected)
        if (std::abs(array.at(index++).toDouble() - value) > 1e-3) return false;
    return true;
}

QJsonObject everyKindSnapshot(int revision) {
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

QJsonObject sharedStatus() {
    return QJsonObject{{"repository", true}, {"branch", "main"}, {"upstream", "origin/main"},
                       {"remote", "git@github.com:jun/film.git"}, {"ahead", 2}, {"behind", 1}, {"changed", false},
                       {"conflicted", false}, {"branches", QJsonArray{"main", "color"}}, {"lfs", true},
                       {"gh", true}, {"live", false}, {"busy", false}};
}

void answerInput(QWidget *parent, const QString &text) {
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

bool popupShowsEveryRow(QComboBox *combo) {
    combo->showPopup();
    QAbstractItemView *list = combo->view();
    bool scrollers = false;
    for (const QWidget *child : list->window()->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
        scrollers = scrollers || (child->inherits("QComboBoxPrivateScroller") && child->isVisible());
    const bool fits = !scrollers && list->verticalScrollBar()->maximum() == 0;
    combo->hidePopup();
    return fits;
}

bool isGraphite(const QColor &color) {
    const bool neutral = std::abs(color.red() - color.green()) <= 2 && std::abs(color.green() - color.blue()) <= 2;
    return neutral && color.alpha() == 255 && color.lightness() <= 0x36 && color.rgb() != QColor(Qt::black).rgb();
}

bool isAmber(const QColor &color) {
    return color.hue() >= 30 && color.hue() <= 52 && color.saturation() > 140 && color.value() > 150;
}

int amberPixels(const QImage &image, QPoint *first) {
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

bool isWaveInk(const QColor &pixel) {
    const bool neutral = std::abs(pixel.red() - pixel.green()) <= 6 && std::abs(pixel.green() - pixel.blue()) <= 6;
    return neutral && pixel.lightness() >= 100 && pixel.lightness() <= 160;
}

QRgb dominantColor(const QImage &image) {
    QHash<QRgb, int> counts;
    for (int y = 0; y < image.height(); y += 2)
        for (int x = 0; x < image.width(); x += 2) ++counts[image.pixel(x, y)];
    QRgb best = 0;
    int most = 0;
    for (auto it = counts.cbegin(); it != counts.cend(); ++it)
        if (it.value() > most) { most = it.value(); best = it.key(); }
    return best;
}

bool nearColor(QRgb a, QRgb b, int tolerance) {
    return std::abs(qRed(a) - qRed(b)) <= tolerance && std::abs(qGreen(a) - qGreen(b)) <= tolerance &&
           std::abs(qBlue(a) - qBlue(b)) <= tolerance;
}

QColor laidOn(const QColor &under, QColor top) {
    const qreal alpha = top.alphaF();
    top.setAlpha(255);
    return mixColor(under, top, alpha);
}

QRgb ink(const QIcon &icon, QIcon::Mode mode, QIcon::State state) {
    const QImage image = icon.pixmap(QSize(20, 20), 2.0, mode, state).toImage();
    QColor best(0, 0, 0, 0);
    for (int y = 0; y < image.height(); ++y)
        for (int x = 0; x < image.width(); ++x)
            if (image.pixelColor(x, y).alpha() > best.alpha()) best = image.pixelColor(x, y);
    return best.rgb();
}

QJsonObject showcaseSnapshot(int revision, const QString &projection) {
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
    // The engine's open proposals as kerfd lists them (plan.md section 3);
    // pendingProposal stays the first of them.
    QJsonObject listed = proposal;
    listed.insert(QStringLiteral("version"), 1);
    listed.insert(QStringLiteral("baseRevision"), revision);
    listed.insert(QStringLiteral("freshness"), QStringLiteral("fresh"));
    listed.insert(QStringLiteral("author"), QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}});
    return QJsonObject{{"revision", revision}, {"duration", 18}, {"projectionStatus", projection},
                       {"timeline", timeline(false)}, {"pendingProposal", proposal}, {"proposalTimeline", timeline(true)},
                       {"proposals", QJsonArray{listed}}, {"proposalTimelines", QJsonObject{{"p7", timeline(true)}}}};
}

void fakeArtwork(MediaArtwork *artwork) {
    static const QImage wide = testing::footageFrame(testing::Shot::HarborWide, QSize(160, 90));
    static const QImage close = testing::footageFrame(testing::Shot::FisherClose, QSize(160, 90));
    static const QImage drone = testing::footageFrame(testing::Shot::DronePass, QSize(160, 90));
    artwork->frames.insert(QStringLiteral("wide"), wide);
    artwork->frames.insert(QStringLiteral("close"), close);
    artwork->frames.insert(QStringLiteral("drone"), drone);
    const auto pan = [](testing::Shot shot) {
        const QImage across = testing::footageFrame(shot, QSize(171 + 15 * 20, MediaArtwork::kStripHeight));
        Filmstrip strip;
        strip.interval = 20.0 / 16.0;
        for (int index = 0; index < 16; ++index) strip.frames << across.copy(index * 20, 0, 171, MediaArtwork::kStripHeight);
        return strip;
    };
    static const Filmstrip wideStrip = pan(testing::Shot::HarborWide);
    static const Filmstrip closeStrip = pan(testing::Shot::FisherClose);
    static const Filmstrip droneStrip = pan(testing::Shot::DronePass);
    artwork->strips.insert(QStringLiteral("wide"), wideStrip);
    artwork->strips.insert(QStringLiteral("close"), closeStrip);
    artwork->strips.insert(QStringLiteral("drone"), droneStrip);
    for (const char *id : {"wide", "close", "score"}) {
        AudioPeaks peaks;
        peaks.levels.append(QVector<AudioPeaks::Bucket>(40 * AudioPeaks::kBucketsPerSecond));
        const double seed = qHash(QByteArray(id)) % 97;
        for (int index = 0; index < peaks.levels[0].size(); ++index) {
            const double x = index * 0.256;
            const double level = 0.06 + 0.5 * std::pow(std::abs(std::sin(x * 0.013 + seed) * std::sin(x * 0.0021 + seed * 0.5)), 2.0);
            const double grain = 0.7 + 0.3 * std::abs(std::sin(index * 1.7 + seed));
            peaks.levels[0][index] = {float(-level * grain), float(level * grain * 0.95), float(level * grain * 0.45)};
        }
        peaks.buildLevels();
        artwork->waves.insert(QString::fromLatin1(id), MediaArtwork::waveImage(peaks));
        artwork->peaks.insert(QString::fromLatin1(id), peaks);
    }
    emit artwork->changed();
}

AudioPeaks fullScalePeaks(double seconds) {
    AudioPeaks peaks;
    peaks.levels.append(QVector<AudioPeaks::Bucket>(qsizetype(seconds * AudioPeaks::kBucketsPerSecond), {-1.0f, 1.0f, 1.0f}));
    peaks.buildLevels();
    return peaks;
}

} // namespace kerf::test
