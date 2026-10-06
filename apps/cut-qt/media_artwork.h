#pragma once

#include <QHash>
#include <QIcon>
#include <QImage>
#include <QJsonArray>
#include <QProcess>
#include <QQueue>
#include <QSet>
#include <QTimer>

namespace cut {

// Decode visual media references outside the paint path. Missing FFmpeg or an
// unsupported source leaves a plain clip; editing never waits for decoration.
class MediaArtwork final : public QObject {
    Q_OBJECT
public:
    explicit MediaArtwork(QObject *parent = nullptr);
    ~MediaArtwork() override;
    void request(const QJsonArray &sources, const QString &directory);
    QHash<QString, QImage> frames;
    QHash<QString, QImage> waves;
    QIcon icon(const QString &id) const;
signals:
    void changed();
private:
    struct Job { QString id; QString path; bool wave; };
    void next();
    QProcess m_process;
    QTimer m_timeout;
    QQueue<Job> m_queue;
    Job m_current;
    QSet<QString> m_requested;
};

} // namespace cut
