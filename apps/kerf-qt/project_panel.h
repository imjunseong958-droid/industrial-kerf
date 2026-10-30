#pragma once

#include <QHash>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QWidget>

class QListWidget;

namespace kerf {

class MediaArtwork;

// The project's media, as a bin of thumbnails. Opening an item loads it into
// the source monitor; items whose file is missing, not downloaded, or
// outside the project folder carry a badge saying so.
class ProjectPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit ProjectPanel(MediaArtwork *artwork, QWidget *parent = nullptr);
    void setSources(const QJsonArray &sources);
    // A `media_status` event: {missing, pointers, external} media ids.
    void setMediaStatus(const QJsonObject &status);
    // OFFLINE, NOT DOWNLOADED, EXTERNAL, or empty.
    QString badge(const QString &mediaId) const { return m_badges.value(mediaId); }

signals:
    void openRequested(const QString &mediaId);
    // A right-click, on an item (its media id) or on the empty bin.
    void menuRequested(const QString &mediaId, const QPoint &globalPosition);

private:
    void refresh();
    // Redraw every tile, for a change of appearance.
    void refreshTiles();
    QIcon tileFor(const QJsonObject &source) const;

    MediaArtwork *m_artwork = nullptr;
    QJsonArray m_sources;
    QHash<QString, QString> m_badges;
    QListWidget *m_list = nullptr;
};

} // namespace kerf
