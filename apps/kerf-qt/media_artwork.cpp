#include "media_artwork.h"

#include <QDir>
#include <QFileInfo>
#include <QJsonObject>
#include <QPainter>
#include <QPolygonF>
#include <QtEndian>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace kerf {

double AudioPeaks::bucketSeconds(int level) {
    return std::pow(double(kMerge), level) / kBucketsPerSecond;
}

AudioPeaks::Bucket AudioPeaks::merge(const Bucket *buckets, qsizetype count) {
    if (count <= 0) return {};
    Bucket merged = buckets[0];
    double squares = double(buckets[0].rms) * buckets[0].rms;
    for (qsizetype index = 1; index < count; ++index) {
        merged.min = std::min(merged.min, buckets[index].min);
        merged.max = std::max(merged.max, buckets[index].max);
        squares += double(buckets[index].rms) * buckets[index].rms;
    }
    merged.rms = float(std::sqrt(squares / double(count)));
    return merged;
}

void AudioPeaks::buildLevels() {
    if (isEmpty()) return;
    levels.resize(1);
    while (levels.last().size() > 1) {
        const QVector<Bucket> &finer = levels.last();
        QVector<Bucket> coarser;
        coarser.reserve((finer.size() + kMerge - 1) / kMerge);
        for (qsizetype index = 0; index < finer.size(); index += kMerge)
            coarser.append(merge(finer.constData() + index, std::min<qsizetype>(kMerge, finer.size() - index)));
        levels.append(std::move(coarser));
    }
}

AudioPeaks::Bucket AudioPeaks::range(double from, double to) const {
    if (isEmpty()) return {};
    if (to < from) std::swap(from, to);
    const double span = to - from;
    int level = 0;
    while (level + 1 < levels.size() && bucketSeconds(level + 1) <= span) ++level;
    const QVector<Bucket> &buckets = levels.at(level);
    const double size = bucketSeconds(level);
    if (span < size) {
        const double at = (from + to) / 2.0 / size - 0.5;
        const double floor = std::floor(at);
        const auto bucketAt = [&buckets](double index) {
            return index >= 0.0 && index < double(buckets.size()) ? buckets.at(qsizetype(index)) : Bucket{};
        };
        const Bucket before = bucketAt(floor);
        const Bucket after = bucketAt(floor + 1.0);
        const float weight = float(at - floor);
        const auto mix = [weight](float a, float b) { return a + (b - a) * weight; };
        return {mix(before.min, after.min), mix(before.max, after.max), mix(before.rms, after.rms)};
    }
    const qsizetype first = std::max<qsizetype>(0, qsizetype(std::floor(from / size)));
    const qsizetype last = std::min<qsizetype>(buckets.size(), qsizetype(std::ceil(to / size)));
    return first < last ? merge(buckets.constData() + first, last - first) : Bucket{};
}

PeakReducer::PeakReducer(int sampleRate) : m_perBucket(std::max(1, sampleRate / AudioPeaks::kBucketsPerSecond)) {}

void PeakReducer::add(const float *samples, qsizetype count) {
    for (qsizetype index = 0; index < count; ++index) {
        const float sample = samples[index];
        if (m_count == 0) {
            m_min = m_max = sample;
        } else {
            m_min = std::min(m_min, sample);
            m_max = std::max(m_max, sample);
        }
        m_squares += double(sample) * sample;
        if (++m_count == m_perBucket) flush();
    }
}

void PeakReducer::flush() {
    m_buckets.append({m_min, m_max, float(std::sqrt(m_squares / m_count))});
    m_count = 0;
    m_squares = 0.0;
}

AudioPeaks PeakReducer::finish() {
    if (m_count > 0) flush();
    AudioPeaks peaks;
    if (!m_buckets.isEmpty()) {
        peaks.levels.append(std::move(m_buckets));
        peaks.buildLevels();
    }
    m_buckets = {};
    return peaks;
}

double Filmstrip::aspect() const {
    if (frames.isEmpty() || frames.first().height() <= 0) return 16.0 / 9.0;
    return double(frames.first().width()) / frames.first().height();
}

int Filmstrip::frameAt(double seconds) const {
    if (frames.isEmpty()) return -1;
    if (interval <= 0.0) return 0;
    const double index = std::round(seconds / interval);
    return int(std::clamp(index, 0.0, double(frames.size() - 1)));
}

void drawWaveform(QPainter &painter, const QRectF &rect, const AudioPeaks &peaks, double from, double to,
                  const QColor &peak, const QColor &body, double gain, const QRectF &visible) {
    if (peaks.isEmpty() || rect.width() <= 0.0 || rect.height() <= 0.0) return;
    const qreal ratio = painter.device() ? std::max<qreal>(1.0, painter.device()->devicePixelRatioF()) : 1.0;
    const qreal step = 1.0 / ratio;
    qreal left = rect.left();
    qreal right = rect.right();
    if (!visible.isNull()) {
        left = std::max(left, visible.left());
        right = std::min(right, visible.right() + 1.0);
    }
    // Columns on the device's pixel grid, so each is one pixel of the screen.
    left = std::max(rect.left(), std::floor(left * ratio) / ratio);
    if (right <= left) return;
    const double perX = (to - from) / rect.width();
    const qreal centre = rect.center().y();
    const qreal half = rect.height() / 2.0;
    // On a square-root scale, as the previous pictures were and as editors
    // show sound: quiet speech still reads beside a loud cue.
    const auto shape = [gain](float value) {
        const double level = std::clamp(double(value) * gain, -1.0, 1.0);
        return std::copysign(std::sqrt(std::abs(level)), level);
    };
    const int columns = int(std::ceil((right - left) * ratio));
    QPolygonF envelope(2 * columns + 4);
    QPolygonF core(2 * columns + 4);
    const int last = 2 * columns + 3;
    for (int column = 0; column < columns; ++column) {
        const qreal x = left + column * step;
        const double seconds = from + (x - rect.left()) * perX;
        const AudioPeaks::Bucket bucket = peaks.range(seconds, seconds + step * perX);
        const double high = shape(bucket.max);
        const double low = shape(bucket.min);
        const double rms = shape(bucket.rms);
        const qreal middle = x + step / 2.0;
        envelope[column + 1] = QPointF(middle, centre - high * half);
        envelope[last - column - 1] = QPointF(middle, centre - low * half);
        core[column + 1] = QPointF(middle, centre - std::min(rms, high) * half);
        core[last - column - 1] = QPointF(middle, centre - std::max(-rms, low) * half);
    }
    // Out to the columns' outer edges, not their middles.
    const qreal end = left + columns * step;
    for (QPolygonF *polygon : {&envelope, &core}) {
        QPolygonF &points = *polygon;
        points[0] = QPointF(left, points[1].y());
        points[columns + 1] = QPointF(end, points[columns].y());
        points[columns + 2] = QPointF(end, points[columns + 3].y());
        points[last] = QPointF(left, points[last - 1].y());
    }
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setPen(Qt::NoPen);
    painter.setBrush(peak);
    painter.drawPolygon(envelope);
    painter.setBrush(body);
    painter.drawPolygon(core);
    painter.restore();
}

QVector<QImage> splitPngStream(const QByteArray &stream) {
    static const QByteArray signature("\x89PNG\r\n\x1a\n", 8);
    QVector<QImage> images;
    qsizetype start = stream.indexOf(signature);
    while (start >= 0) {
        // Chunk by chunk (length, type, data, CRC) to the IEND that closes it.
        qsizetype at = start + signature.size();
        bool closed = false;
        while (!closed && at + 12 <= stream.size()) {
            const quint32 length = qFromBigEndian<quint32>(stream.constData() + at);
            closed = std::memcmp(stream.constData() + at + 4, "IEND", 4) == 0;
            at += 12 + qsizetype(length);
        }
        if (!closed || at > stream.size()) break;
        const QImage image = QImage::fromData(QByteArrayView(stream).sliced(start, at - start), "PNG");
        if (!image.isNull()) images.append(image);
        start = stream.indexOf(signature, at);
    }
    return images;
}

QImage MediaArtwork::waveImage(const AudioPeaks &peaks) {
    QImage image(2048, 48, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    // The envelope at part strength: inked in one colour, the RMS still
    // reads inside it.
    drawWaveform(painter, QRectF(image.rect()), peaks, 0.0, peaks.duration(), QColor(255, 255, 255, 140), Qt::white);
    return image;
}

int MediaArtwork::stripFrameCount(double duration) {
    // Eight a second covers the closest zoom's tiles; a long source shares
    // its frames out more thinly.
    if (duration <= 0.0) return kStripFrames;
    return std::clamp(int(std::ceil(duration * 8.0)), 1, kStripFrames);
}

MediaArtwork::MediaArtwork(QObject *parent) : QObject(parent) {
    connect(&m_process, &QProcess::readyReadStandardOutput, this, &MediaArtwork::readSamples);
    connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        jobFinished(code == 0 && status == QProcess::NormalExit);
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error != QProcess::FailedToStart) return;
        m_timeout.stop();
        m_reducer.reset();
        // Not from inside start(): without FFmpeg every job fails at once.
        QMetaObject::invokeMethod(this, &MediaArtwork::next, Qt::QueuedConnection);
    });
    m_timeout.setSingleShot(true);
    connect(&m_timeout, &QTimer::timeout, &m_process, &QProcess::kill);
}

MediaArtwork::~MediaArtwork() {
    disconnect(&m_process, nullptr, this, nullptr);
    m_timeout.stop();
    m_process.kill();
    m_process.waitForFinished(1000);
}

void MediaArtwork::request(const QJsonArray &sources, const QString &directory) {
    for (const auto &value : sources) {
        const auto source = value.toObject();
        const QString id = source.value("id").toString();
        const QString path = QDir(directory).absoluteFilePath(source.value("path").toString());
        const QString key = id + path + QString::number(QFileInfo(path).lastModified().toMSecsSinceEpoch());
        if (m_requested.contains(key)) continue;
        m_requested.insert(key);
        m_keys.insert(id, key);
        frames.remove(id); waves.remove(id); peaks.remove(id); strips.remove(id);
        const double duration = source.value("duration").toDouble();
        const bool audio = source.value("hasAudio").toBool();
        // A source that says it is sound alone has no picture to look for.
        if (source.contains("width") || !audio) {
            m_queues[Poster].enqueue({id, path, key, Poster, duration});
            m_queues[Strip].enqueue({id, path, key, Strip, duration});
        }
        if (audio) m_queues[Peaks].enqueue({id, path, key, Peaks, duration});
    }
    if (!m_busy) next();
}

void MediaArtwork::readSamples() {
    if (!m_reducer) return;
    m_partialSample += m_process.readAllStandardOutput();
    const qsizetype count = m_partialSample.size() / qsizetype(sizeof(float));
    if (count == 0) return;
    // Copied out, as the bytes need not be aligned for floats.
    QVector<float> samples(count);
    std::memcpy(samples.data(), m_partialSample.constData(), count * sizeof(float));
    m_reducer->add(samples.constData(), count);
    m_partialSample.remove(0, count * qsizetype(sizeof(float)));
}

void MediaArtwork::jobFinished(bool succeeded) {
    m_timeout.stop();
    const Job job = m_current;
    // A result for a file replaced since it was asked for is dropped.
    const bool current = m_keys.value(job.id) == job.key;
    if (job.kind == Peaks) {
        readSamples();
        AudioPeaks result = m_reducer ? m_reducer->finish() : AudioPeaks();
        m_reducer.reset();
        m_partialSample.clear();
        if (succeeded && current && !result.isEmpty()) {
            waves.insert(job.id, waveImage(result));
            peaks.insert(job.id, std::move(result));
            emit changed();
        }
    } else {
        const QByteArray output = m_process.readAllStandardOutput();
        if (succeeded && current && job.kind == Poster) {
            const QImage poster = QImage::fromData(output);
            if (!poster.isNull()) {
                frames.insert(job.id, poster);
                emit changed();
            }
        } else if (succeeded && current && job.kind == Strip) {
            Filmstrip strip{splitPngStream(output), m_interval};
            if (!strip.isEmpty()) {
                strips.insert(job.id, std::move(strip));
                emit changed();
            }
        }
    }
    next();
}

void MediaArtwork::next() {
    m_busy = false;
    for (QQueue<Job> &queue : m_queues) {
        while (!queue.isEmpty()) {
            Job job = queue.dequeue();
            if (m_keys.value(job.id) != job.key) continue;
            m_current = std::move(job);
            m_busy = true;
            break;
        }
        if (m_busy) break;
    }
    if (!m_busy) return;
    const double duration = std::max(0.0, m_current.duration);
    QStringList args{"-nostdin", "-v", "error"};
    // Anamorphic pictures are squared up first, at their display shape.
    const QString square = QStringLiteral("scale=trunc(iw*sar/2)*2:ih,setsar=1");
    double seconds = 30.0;
    if (m_current.kind == Poster) {
        // Scaled before the thumbnail filter looks for a representative
        // frame, so it holds a hundred small frames rather than 4K ones.
        args << "-i" << m_current.path << "-an" << "-sn" << "-vf"
             << QStringLiteral("%1,scale=%2:%3:force_original_aspect_ratio=increase:flags=lanczos,thumbnail")
                    .arg(square).arg(kPosterSize.width()).arg(kPosterSize.height())
             << "-frames:v" << "1" << "-f" << "image2pipe" << "-vcodec" << "png" << "pipe:1";
    } else if (m_current.kind == Strip) {
        const int count = stripFrameCount(duration);
        m_interval = duration > 0.0 ? duration / count : 1.0;
        // Frames further apart than a second come from keyframes alone: a
        // long source then takes seconds, not a decode of every frame, and
        // the nearest keyframe stands in well at that spacing.
        if (m_interval > 1.0) args << "-skip_frame" << "nokey";
        args << "-i" << m_current.path << "-an" << "-sn" << "-vf"
             << QStringLiteral("fps=%1,%2,scale=-2:%3:flags=lanczos").arg(1.0 / m_interval, 0, 'g', 10).arg(square).arg(kStripHeight)
             << "-frames:v" << QString::number(count) << "-f" << "image2pipe" << "-vcodec" << "png" << "pipe:1";
        seconds += duration * (m_interval > 1.0 ? 0.25 : 1.0);
    } else {
        // Raw samples through the pipe, reduced as they arrive.
        m_reducer.emplace(kPeakSampleRate);
        m_partialSample.clear();
        args << "-i" << m_current.path << "-vn" << "-sn" << "-ac" << "1" << "-ar" << QString::number(kPeakSampleRate)
             << "-f" << "f32le" << "-c:a" << "pcm_f32le" << "pipe:1";
        seconds += duration * 0.25;
    }
    m_process.start(QStringLiteral("ffmpeg"), args);
    m_timeout.start(int(std::min(seconds, 900.0) * 1000.0));
}

} // namespace kerf
