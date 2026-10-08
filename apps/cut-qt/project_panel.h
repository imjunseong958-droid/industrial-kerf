#pragma once

#include <QJsonArray>
#include <QWidget>

class QListWidget;

namespace cut {

class MediaArtwork;

// The project's media, as a bin of thumbnails. Opening an item loads it into
// the source monitor.
class ProjectPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit ProjectPanel(MediaArtwork *artwork, QWidget *parent = nullptr);
    void setSources(const QJsonArray &sources);

signals:
    void openRequested(const QString &mediaId);

private:
    void refresh();

    MediaArtwork *m_artwork = nullptr;
    QJsonArray m_sources;
    QListWidget *m_list = nullptr;
};

} // namespace cut
