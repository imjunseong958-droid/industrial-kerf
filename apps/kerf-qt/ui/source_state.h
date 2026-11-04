#pragma once

// The Source viewer's model (Q-SOURCE): one piece of media from the bin, its
// position, its in and out marks, the decoded frame on screen and the FFmpeg
// processes that decode it. The Inspector sheet's Source page shows it.
//
// Frames decode in the background, the latest request winning. Play streams
// the picture (no sound: the bridge plays only the program) from FFmpeg at
// the media's own rate.

#include "test_friends.h"

#include <QByteArray>
#include <QHash>
#include <QImage>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QString>
#include <QVector>

class QAction;

namespace kerf {

class MediaArtwork;

class SourceState final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit SourceState(QObject *parent = nullptr);
    ~SourceState() override;

    // `source` is a snapshot media entry; `path` is resolved against the project.
    void open(const QJsonObject &source, const QString &path);
    // The open media again, from a newer snapshot: its entry, and a new file
    // when it was replaced (the marks stay, within its length).
    void refresh(const QJsonObject &source, const QString &path);
    void close();
    bool isOpen() const { return !m_mediaId.isEmpty(); }
    QString mediaId() const { return m_mediaId; }
    QJsonObject source() const { return m_source; }
    QString path() const { return m_path; }
    // The media as people know it (sourceName), empty when closed.
    QString name() const;
    double duration() const { return m_duration; }
    double fps() const { return m_fps; }
    double position() const { return m_position; }
    double inPoint() const { return m_in; }
    double outPoint() const { return m_out; }
    bool hasVideo() const { return m_hasVideo; }
    // A range narrower than the whole media is marked.
    bool isMarked() const;
    // The decoded picture on screen; null until the first frame arrives.
    QImage frame() const { return m_frame; }
    // What the picture says while it has no frame.
    QString message() const { return m_message; }

    void seek(double seconds);
    void step(int frames);
    void markIn();
    void markOut();
    void clearMarks();
    // Both marks at once (a shot's row), snapped to frames within the media.
    void setRange(double in, double out);

    // Play streams the picture at the media's rate from the position (from
    // the in mark when at the out mark or the end), and stops at the out
    // mark when it started inside the marked range, else at the end. Only
    // media with a picture plays.
    bool canPlay() const { return isOpen() && m_hasVideo; }
    bool isPlaying() const { return m_playing; }
    void play();
    void pause();
    void togglePlay();

    // ---- around it ---------------------------------------------------------
    // The program clip "Open in timeline" goes to; empty when the media is
    // not in the timeline.
    void setRevealClip(const QString &clip);
    QString revealClip() const { return m_revealClip; }
    // The window's artwork: the waveform of sound.
    void setArtwork(MediaArtwork *artwork);
    MediaArtwork *artwork() const;
    // The registry's commands the viewers' buttons run, by command id
    // (`source.markIn` ...). Without one a button acts on this state itself
    // or, for an edit, stays disabled.
    void setActions(const QHash<QString, QAction *> &actions);
    void addActions(const QHash<QString, QAction *> &actions);
    QAction *action(const QString &id) const;
    QList<QAction *> actions() const;

    // The size Play streams at for a `width` x `height` picture: at most 640
    // wide, both even.
    static QSize streamSize(int width, int height);
    // A media rate as the snapshot prints it: `24`, `29.97`, or `30000/1001`.
    static double parseRate(const QString &rate);

signals:
    // Another media was opened, or the open one closed (empty).
    void opened(const QString &media);
    // The position, the marks, or what is open changed.
    void changed();
    void frameChanged();
    void playingChanged(bool playing);
    void revealChanged();
    void artworkChanged();
    void actionsChanged();

private:
    void decode(double seconds);
    void decoded(int exitCode, QProcess::ExitStatus status);
    void stopPlayer();
    void readStream();
    void playerFinished();
    void settleStill();
    double lastFrameStart() const;
    double snap(double seconds) const;

    QJsonObject m_source;
    QString m_mediaId;
    QString m_path;
    double m_duration = 0.0;
    double m_fps = 24.0;
    double m_position = 0.0;
    double m_in = 0.0;
    double m_out = 0.0;
    bool m_hasVideo = false;
    QImage m_frame;
    QString m_message;

    QProcess m_decoder;
    // The file the running decode reads.
    QString m_decoding;
    bool m_hasPending = false;
    double m_pending = 0.0;

    QProcess m_player;
    bool m_playing = false;
    double m_playStart = 0.0;
    double m_playStop = 0.0;
    qint64 m_playFrames = 0;
    QSize m_streamSize;
    QByteArray m_stream;


    QString m_revealClip;
    QPointer<MediaArtwork> m_artwork;
    QHash<QString, QPointer<QAction>> m_actions;
};

} // namespace kerf
