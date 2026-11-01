#pragma once

#include <QByteArray>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QProcess>
#include <QQueue>
#include <QRectF>
#include <QSet>
#include <QTimer>
#include <QVector>

#include <optional>

class QPainter;

namespace kerf {

// A source's sound as data, not a picture: the lowest and highest sample and
// the RMS of every 5 ms, mixed to mono, so a waveform is drawn from the
// samples under each pixel and stays crisp at any zoom. Each coarser level
// merges four buckets of the one below, so a column spanning minutes reads a
// handful of buckets, and painting costs what the clip's pixels do, not
// what its length does.
struct AudioPeaks {
    static constexpr int kBucketsPerSecond = 200;
    static constexpr int kMerge = 4;
    struct Bucket {
        float min = 0.0f;
        float max = 0.0f;
        float rms = 0.0f;
    };
    // levels[0] at kBucketsPerSecond; each next one kMerge times coarser.
    QVector<QVector<Bucket>> levels;

    bool isEmpty() const { return levels.isEmpty() || levels.first().isEmpty(); }
    double duration() const { return isEmpty() ? 0.0 : double(levels.first().size()) / kBucketsPerSecond; }
    static double bucketSeconds(int level);
    // The extremes and RMS of `from` to `to` seconds, from the coarsest level
    // whose buckets still fit the span. A span shorter than one bucket (a
    // deep zoom) is interpolated between the buckets either side, so it
    // draws a curve rather than steps. Outside the sound it is silence.
    Bucket range(double from, double to) const;
    // Fills in every coarser level from levels[0].
    void buildLevels();
    static Bucket merge(const Bucket *buckets, qsizetype count);
};

// Turns a stream of mono samples into level-0 buckets a chunk at a time, so
// an hour of sound never sits in memory as samples.
class PeakReducer {
public:
    explicit PeakReducer(int sampleRate);
    void add(const float *samples, qsizetype count);
    // The last, partial bucket, then the coarser levels.
    AudioPeaks finish();

private:
    void flush();
    int m_perBucket;
    int m_count = 0;
    float m_min = 0.0f;
    float m_max = 0.0f;
    double m_squares = 0.0;
    QVector<AudioPeaks::Bucket> m_buckets;
};

// Frames sampled evenly along a source, at their own shape (no bars baked
// in): frame k shows the source `k * interval` seconds in.
struct Filmstrip {
    QVector<QImage> frames;
    double interval = 0.0;

    bool isEmpty() const { return frames.isEmpty(); }
    // Width over height of its pictures.
    double aspect() const;
    // The frame nearest `seconds` into the source, or -1 with none.
    int frameAt(double seconds) const;
};

// Paints `peaks` from `from` to `to` seconds of the source across `rect`, one
// column per device pixel and only where `visible` (when given) meets it: an
// envelope of the extremes in `peak`, and the RMS inside it in `body`, either
// side of the centre line, scaled by the linear `gain`. `to` before `from`
// draws the sound reversed.
void drawWaveform(QPainter &painter, const QRectF &rect, const AudioPeaks &peaks, double from, double to,
                  const QColor &peak, const QColor &body, double gain = 1.0, const QRectF &visible = QRectF());

// The PNG images FFmpeg's image2pipe writes back to back on one stream.
QVector<QImage> splitPngStream(const QByteArray &stream);

// Decode visual media references outside the paint path. Missing FFmpeg or an
// unsupported source leaves a plain clip; editing never waits for decoration.
class MediaArtwork final : public QObject {
    Q_OBJECT
public:
    // A poster covers the bin's tile at twice its size, so it is sharp
    // fitted there on a Retina screen and filled into the Inspector's.
    static constexpr QSize kPosterSize{248, 140};
    // Filmstrip frames are this tall, a pixel for a pixel of the Large
    // preset's body on a Retina screen, and at most this many along a source.
    static constexpr int kStripHeight = 96;
    static constexpr int kStripFrames = 64;
    // Peaks are taken from sound resampled to this rate.
    static constexpr int kPeakSampleRate = 16000;

    explicit MediaArtwork(QObject *parent = nullptr);
    ~MediaArtwork() override;
    void request(const QJsonArray &sources, const QString &directory);
    // Each picture source's poster: one representative frame, at its own shape.
    QHash<QString, QImage> frames;
    // Each sound's peaks, and frames along each picture.
    QHash<QString, AudioPeaks> peaks;
    QHash<QString, Filmstrip> strips;
    // The whole sound drawn in white on clear, 2048 by 48, for views that
    // ink a picture of it (the Inspector's thumbnail).
    QHash<QString, QImage> waves;
    static QImage waveImage(const AudioPeaks &peaks);
    // How many frames a strip of a `duration` second source takes, spread
    // evenly along it.
    static int stripFrameCount(double duration);
signals:
    void changed();
private:
    // Drained in this order: every poster first, so the bin fills fast, then
    // the sounds, then the filmstrips.
    enum Kind { Poster, Peaks, Strip, KindCount };
    struct Job {
        QString id;
        QString path;
        QString key;
        Kind kind = Poster;
        double duration = 0.0;
        // A sought strip's frame, from the first.
        int sample = 0;
    };
    // Sources longer than this have their strip sought a frame at a time;
    // shorter ones are decoded once.
    static constexpr double kDecodedStripSeconds = 16.0;
    static bool seeksStrip(double duration);
    void stripFrameFinished(const Job &job, const QImage &frame, bool current);
    void next();
    void jobFinished(bool succeeded);
    void readSamples();
    QProcess m_process;
    QTimer m_timeout;
    QQueue<Job> m_queues[KindCount];
    Job m_current;
    bool m_busy = false;
    // The current strip's spacing, and the sound being reduced as it streams in.
    double m_interval = 0.0;
    std::optional<PeakReducer> m_reducer;
    QByteArray m_partialSample;
    // A sought strip's frames so far, by id.
    QHash<QString, QVector<QImage>> m_stripFrames;
    // The key (id, path and modification time) each id was last asked for
    // under, so a result for a file since replaced is dropped.
    QHash<QString, QString> m_keys;
};

} // namespace kerf
