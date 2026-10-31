#pragma once

#include <QHash>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QWidget>

class QLineEdit;
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
    // The grid at 1440x900 (a column of 300): two columns of 124 by 70
    // tiles, 10 apart, 16 in from the column's edge, with room for a
    // scroll bar on the right.
    static constexpr QSize kThumbnail{124, 70};
    static constexpr int kSidePadding = 16;
    static constexpr int kTileGap = 10;
    static constexpr int kTileInset = 4;
    explicit ProjectPanel(MediaArtwork *artwork, QWidget *parent = nullptr);
    void setSources(const QJsonArray &sources);
    // A `media_status` event: {missing, pointers, external} media ids.
    void setMediaStatus(const QJsonObject &status);
    // Offline, Not downloaded, External, or empty.
    QString badge(const QString &mediaId) const { return m_badges.value(mediaId); }
    QLineEdit *search() const { return m_search; }

signals:
    void openRequested(const QString &mediaId);
    // A right-click, on an item (its media id) or on the empty bin.
    void menuRequested(const QString &mediaId, const QPoint &globalPosition);

private:
    void refresh();
    // Redraw every tile, for a change of appearance.
    void refreshTiles();
    QIcon tileFor(const QJsonObject &source) const;
    // Show only the media whose name holds `text`.
    void applyFilter(const QString &text);
    static QIcon searchGlyph();

    MediaArtwork *m_artwork = nullptr;
    QJsonArray m_sources;
    QHash<QString, QString> m_badges;
    QLineEdit *m_search = nullptr;
    QListWidget *m_list = nullptr;
};

} // namespace kerf
