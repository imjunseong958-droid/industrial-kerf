#include "source_state.h"

#include "editor_common.h"
#include "media_artwork.h"

#include <QAction>
#include <QJsonArray>
#include <QSignalBlocker>
#include <QStringList>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

const QString kNoMedia = QStringLiteral("Double-click media in the bin to open it here");

QStringList frameArguments(double seconds, const QString &path) {
    return {QStringLiteral("-nostdin"), QStringLiteral("-v"), QStringLiteral("error"),
            QStringLiteral("-ss"), QString::number(seconds, 'f', 6),
            QStringLiteral("-i"), path, QStringLiteral("-frames:v"), QStringLiteral("1"),
            QStringLiteral("-vf"), QStringLiteral("scale=960:-2"),
            QStringLiteral("-f"), QStringLiteral("image2pipe"), QStringLiteral("-vcodec"),
            QStringLiteral("png"), QStringLiteral("pipe:1")};
}

} // namespace

SourceState::SourceState(QObject *parent) : QObject(parent), m_message(kNoMedia) {
    setObjectName(QStringLiteral("sourceState"));
    connect(&m_decoder, &QProcess::finished, this, &SourceState::decoded);
    connect(&m_decoder, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error != QProcess::FailedToStart) return;
        m_message = QStringLiteral("FFmpeg is needed to show source frames");
        emit frameChanged();
    });
    connect(&m_player, &QProcess::readyReadStandardOutput, this, &SourceState::readStream);
    connect(&m_player, &QProcess::finished, this, &SourceState::playerFinished);
    connect(&m_player, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) playerFinished();
    });
}

SourceState::~SourceState() {
    // Nothing is told of the processes ending: the state is going.
    m_decoder.disconnect(this);
    m_player.disconnect(this);
    for (QProcess *process : {&m_decoder, &m_player}) {
        process->kill();
        process->waitForFinished(1000);
    }
}

double SourceState::parseRate(const QString &rate) {
    const QStringList parts = rate.split(QLatin1Char('/'));
    const double value = parts.size() == 2 && parts.at(1).toDouble() > 0.0
        ? parts.at(0).toDouble() / parts.at(1).toDouble() : rate.toDouble();
    return value > 0.0 ? value : 24.0;
}

QSize SourceState::streamSize(int width, int height) {
    if (width <= 0 || height <= 0) return {};
    const int w = std::max(2, std::min(640, width) / 2 * 2);
    const int h = std::max(2, int(std::lround(double(w) * height / width / 2.0)) * 2);
    return {w, h};
}

QString SourceState::name() const {
    return isOpen() ? sourceName(m_source) : QString();
}

bool SourceState::isMarked() const {
    if (!isOpen()) return false;
    const double half = 0.5 / m_fps;
    return m_in > half || m_out < m_duration - half;
}

double SourceState::lastFrameStart() const {
    return std::max(0.0, m_duration - 1.0 / m_fps);
}

double SourceState::snap(double seconds) const {
    return std::round(seconds * m_fps) / m_fps;
}

void SourceState::open(const QJsonObject &source, const QString &path) {
    stopPlayer();
    m_source = source;
    m_mediaId = source.value(QStringLiteral("id")).toString();
    m_path = path;
    m_duration = std::max(0.0, source.value(QStringLiteral("duration")).toDouble());
    m_fps = parseRate(source.value(QStringLiteral("rate")).toString());
    m_hasVideo = source.contains(QStringLiteral("width"));
    m_in = 0.0;
    m_out = m_duration;
    m_frame = QImage();
    m_message = m_hasVideo ? QStringLiteral("Loading…") : QString();
    m_position = -1.0;
    showIndexFor(m_mediaId);
    emit opened(m_mediaId);
    emit frameChanged();
    seek(0.0);
}

void SourceState::refresh(const QJsonObject &source, const QString &path) {
    if (!isOpen() || source.value(QStringLiteral("id")).toString() != m_mediaId) return;
    const bool replaced = path != m_path;
    m_source = source;
    if (!replaced) return;
    stopPlayer();
    m_path = path;
    m_duration = std::max(0.0, source.value(QStringLiteral("duration")).toDouble());
    m_fps = parseRate(source.value(QStringLiteral("rate")).toString());
    m_hasVideo = source.contains(QStringLiteral("width"));
    m_out = std::min(m_out, m_duration);
    if (m_out <= 0.0) m_out = m_duration;
    m_in = std::min(m_in, std::max(0.0, m_out - 1.0 / m_fps));
    forgetIndex(m_mediaId);
    // The other file's index is not this one's: ask again.
    emit opened(m_mediaId);
    const double at = std::clamp(m_position, 0.0, lastFrameStart());
    m_position = -1.0;
    seek(at);
}

void SourceState::close() {
    stopPlayer();
    m_source = {};
    m_mediaId.clear();
    m_path.clear();
    m_duration = m_position = m_in = m_out = 0.0;
    m_hasVideo = false;
    m_frame = QImage();
    m_message = kNoMedia;
    m_revealClip.clear();
    showIndexFor(QString());
    emit opened(QString());
    emit frameChanged();
    emit revealChanged();
    emit changed();
}

void SourceState::seek(double seconds) {
    if (!isOpen()) return;
    if (m_playing) {
        stopPlayer();
        emit playingChanged(false);
    }
    // Stay on frame starts, before the media's last frame.
    const double target = std::clamp(snap(seconds), 0.0, lastFrameStart());
    if (target == m_position) return;
    m_position = target;
    emit changed();
    if (m_hasVideo) decode(target);
}

void SourceState::step(int frames) {
    seek(std::max(0.0, m_position) + frames / m_fps);
}

void SourceState::markIn() {
    if (!isOpen()) return;
    m_in = std::max(0.0, m_position);
    if (m_out <= m_in) m_out = m_duration;
    emit changed();
}

void SourceState::markOut() {
    if (!isOpen()) return;
    // The out mark includes the frame on screen.
    m_out = std::min(m_duration, std::max(0.0, m_position) + 1.0 / m_fps);
    if (m_in >= m_out) m_in = 0.0;
    emit changed();
}

void SourceState::clearMarks() {
    m_in = 0.0;
    m_out = m_duration;
    emit changed();
}

void SourceState::setRange(double in, double out) {
    if (!isOpen()) return;
    const double first = std::clamp(snap(in), 0.0, lastFrameStart());
    double last = std::clamp(snap(out), 0.0, m_duration);
    if (last <= first) last = std::min(m_duration, first + 1.0 / m_fps);
    m_in = first;
    m_out = last;
    emit changed();
}

// ---------------------------------------------------------------- decoding

void SourceState::decode(double seconds) {
    if (m_decoder.state() != QProcess::NotRunning) {
        m_hasPending = true;
        m_pending = seconds;
        return;
    }
    m_hasPending = false;
    m_decoding = m_path;
    m_decoder.start(QStringLiteral("ffmpeg"), frameArguments(seconds, m_path));
}

void SourceState::decoded(int exitCode, QProcess::ExitStatus status) {
    const QByteArray bytes = m_decoder.readAllStandardOutput();
    // A frame for media since closed or replaced, or one that playing has
    // passed, is dropped; the open media's own frame is asked for instead.
    if (!isOpen() || m_playing || m_decoding != m_path) {
        const bool again = isOpen() && !m_playing && m_hasVideo;
        m_hasPending = false;
        if (again) decode(m_position);
        return;
    }
    if (status == QProcess::NormalExit && exitCode == 0) {
        const QImage image = QImage::fromData(bytes, "PNG");
        if (!image.isNull()) m_frame = image;
    } else if (m_frame.isNull()) {
        m_message = QStringLiteral("This media cannot be shown");
    }
    emit frameChanged();
    if (m_hasPending) decode(m_pending);
}

// ---------------------------------------------------------------- playing

void SourceState::play() {
    if (!canPlay() || m_playing) return;
    const double frame = 1.0 / m_fps;
    double from = std::max(0.0, m_position);
    const bool marked = isMarked();
    // On the range's last frame (where play stops at the out mark, and where
    // Mark out leaves the playhead) or the media's last, play starts over
    // from the in mark.
    const double lastRangeFrame = m_out - frame;
    if ((marked && from >= lastRangeFrame - frame / 2) || from >= lastFrameStart() - frame / 2) from = marked ? m_in : 0.0;
    m_playStart = from;
    m_playStop = marked && from < m_out ? m_out : m_duration;
    m_playFrames = 0;
    m_stream.clear();
    m_streamSize = streamSize(m_source.value(QStringLiteral("width")).toInt(), m_source.value(QStringLiteral("height")).toInt());
    if (m_streamSize.isEmpty()) m_streamSize = QSize(640, 360);
    m_hasPending = false;
    m_playing = true;
    m_player.start(QStringLiteral("ffmpeg"),
                   {QStringLiteral("-nostdin"), QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-re"),
                    QStringLiteral("-ss"), QString::number(from, 'f', 6), QStringLiteral("-i"), m_path,
                    QStringLiteral("-an"), QStringLiteral("-sn"), QStringLiteral("-vf"),
                    QStringLiteral("scale=%1:%2").arg(m_streamSize.width()).arg(m_streamSize.height()),
                    QStringLiteral("-pix_fmt"), QStringLiteral("rgba"), QStringLiteral("-f"), QStringLiteral("rawvideo"),
                    QStringLiteral("pipe:1")});
    if (m_position != from) {
        m_position = from;
        emit changed();
    }
    emit playingChanged(true);
}

void SourceState::pause() {
    if (!m_playing) return;
    stopPlayer();
    emit playingChanged(false);
    settleStill();
}

void SourceState::togglePlay() {
    if (m_playing) pause();
    else play();
}

void SourceState::stopPlayer() {
    m_playing = false;
    m_stream.clear();
    if (m_player.state() == QProcess::NotRunning) return;
    const QSignalBlocker blocker(&m_player);
    m_player.kill();
    m_player.waitForFinished(1000);
}

void SourceState::readStream() {
    if (!m_playing) {
        m_player.readAllStandardOutput();
        return;
    }
    m_stream.append(m_player.readAllStandardOutput());
    const qsizetype bytes = qsizetype(m_streamSize.width()) * m_streamSize.height() * 4;
    if (bytes <= 0) return;
    const double frame = 1.0 / m_fps;
    QImage latest;
    double at = m_position;
    bool done = false;
    while (m_stream.size() >= bytes) {
        const double t = m_playStart + m_playFrames * frame;
        ++m_playFrames;
        if (t > m_playStop - frame / 2) {
            done = true;
            break;
        }
        latest = QImage(reinterpret_cast<const uchar *>(m_stream.constData()), m_streamSize.width(), m_streamSize.height(),
                        m_streamSize.width() * 4, QImage::Format_RGBA8888).copy();
        at = std::clamp(snap(t), 0.0, lastFrameStart());
        m_stream.remove(0, bytes);
    }
    if (!latest.isNull()) {
        m_frame = latest;
        m_position = at;
        emit frameChanged();
        emit changed();
    }
    if (done) {
        stopPlayer();
        emit playingChanged(false);
        settleStill();
    }
}

void SourceState::settleStill() {
    // The streamed frame is smaller than a still: decode the one on screen.
    if (isOpen() && m_hasVideo) decode(m_position);
}

void SourceState::playerFinished() {
    if (!m_playing) return;
    readStream();
    if (!m_playing) return;
    // The stream ended (the media's end, or FFmpeg could not read it).
    m_playing = false;
    m_stream.clear();
    emit playingChanged(false);
    settleStill();
}

// ---------------------------------------------------------------- the media index

QVector<SourceState::Range> SourceState::rangesFromIndex(const QJsonObject &index, double duration) {
    QVector<Range> ranges;
    for (const QJsonValue &value : index.value(QStringLiteral("ranges")).toArray()) {
        double start = 0.0;
        double end = 0.0;
        if (value.isArray()) {
            start = value.toArray().at(0).toDouble();
            end = value.toArray().at(1).toDouble();
        } else {
            const QJsonObject range = value.toObject();
            start = range.value(QStringLiteral("start")).toDouble();
            end = range.value(QStringLiteral("end")).toDouble();
        }
        if (end > start) ranges.append(Range{start, end, {}});
    }
    if (ranges.isEmpty()) {
        QVector<double> cuts;
        for (const QJsonValue &cut : index.value(QStringLiteral("cuts")).toArray()) {
            const double t = cut.toDouble();
            if (t > 0.0 && (duration <= 0.0 || t < duration)) cuts.append(t);
        }
        if (!cuts.isEmpty() && duration > 0.0) {
            std::sort(cuts.begin(), cuts.end());
            cuts.erase(std::unique(cuts.begin(), cuts.end()), cuts.end());
            cuts.prepend(0.0);
            cuts.append(duration);
            for (qsizetype i = 0; i + 1 < cuts.size(); ++i) ranges.append(Range{cuts.at(i), cuts.at(i + 1), {}});
        }
    }
    std::sort(ranges.begin(), ranges.end(), [](const Range &a, const Range &b) { return a.start < b.start; });
    for (qsizetype i = 0; i < ranges.size(); ++i) ranges[i].name = QStringLiteral("Shot %1").arg(i + 1);
    return ranges;
}

bool SourceState::applyIndex(const QJsonObject &event) {
    const QString media = event.value(QStringLiteral("media")).toString();
    if (media.isEmpty()) return false;
    m_indexes.insert(media, event);
    m_indexErrors.remove(media);
    if (media != m_mediaId) return false;
    showIndexFor(media);
    return true;
}

bool SourceState::applyIndexError(const QJsonObject &event) {
    QString media = event.value(QStringLiteral("media")).toString();
    if (media.isEmpty() && m_indexStatus == IndexStatus::Pending) media = m_mediaId;
    if (media.isEmpty()) return false;
    const QString message = event.value(QStringLiteral("message")).toString();
    m_indexErrors.insert(media, message.isEmpty() ? QStringLiteral("This media could not be indexed") : message);
    m_indexes.remove(media);
    if (media != m_mediaId) return false;
    showIndexFor(media);
    return true;
}

void SourceState::setIndexPending() {
    if (!isOpen() || m_indexStatus == IndexStatus::Ready) return;
    m_indexErrors.remove(m_mediaId);
    m_indexStatus = IndexStatus::Pending;
    m_indexMessage = QStringLiteral("Finding shots…");
    emit indexChanged();
}

void SourceState::setIndexUnavailable() {
    if (!isOpen() || m_indexStatus == IndexStatus::Ready) return;
    m_indexStatus = IndexStatus::Unavailable;
    m_indexMessage = QStringLiteral("Shots show once Kerf is connected");
    emit indexChanged();
}

void SourceState::forgetIndex(const QString &media) {
    m_indexes.remove(media);
    m_indexErrors.remove(media);
    if (media == m_mediaId) showIndexFor(media);
}

void SourceState::showIndexFor(const QString &media) {
    m_ranges.clear();
    m_indexMessage.clear();
    if (media.isEmpty()) {
        m_indexStatus = IndexStatus::None;
    } else if (m_indexes.contains(media)) {
        m_indexStatus = IndexStatus::Ready;
        m_ranges = rangesFromIndex(m_indexes.value(media), m_duration);
        if (m_ranges.isEmpty()) m_indexMessage = QStringLiteral("No cuts in this media");
    } else if (m_indexErrors.contains(media)) {
        m_indexStatus = IndexStatus::Failed;
        m_indexMessage = m_indexErrors.value(media);
    } else {
        m_indexStatus = IndexStatus::None;
    }
    emit indexChanged();
}

// ---------------------------------------------------------------- around it

void SourceState::setRevealClip(const QString &clip) {
    if (clip == m_revealClip) return;
    m_revealClip = clip;
    emit revealChanged();
}

void SourceState::setArtwork(MediaArtwork *artwork) {
    if (m_artwork == artwork) return;
    if (m_artwork) m_artwork->disconnect(this);
    m_artwork = artwork;
    if (artwork) connect(artwork, &MediaArtwork::changed, this, &SourceState::artworkChanged);
    emit artworkChanged();
}

MediaArtwork *SourceState::artwork() const {
    return m_artwork;
}

void SourceState::setActions(const QHash<QString, QAction *> &actions) {
    m_actions.clear();
    addActions(actions);
}

void SourceState::addActions(const QHash<QString, QAction *> &actions) {
    for (auto it = actions.cbegin(); it != actions.cend(); ++it) {
        if (!it.value()) continue;
        if (m_actions.value(it.key()) == it.value()) continue;
        m_actions.insert(it.key(), it.value());
    }
    emit actionsChanged();
}

QAction *SourceState::action(const QString &id) const {
    return m_actions.value(id);
}

QList<QAction *> SourceState::actions() const {
    QList<QAction *> list;
    for (const QPointer<QAction> &action : m_actions)
        if (action) list.append(action);
    return list;
}

} // namespace kerf
