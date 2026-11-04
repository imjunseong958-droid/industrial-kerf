#pragma once

// The media bin at the left of the bottom band (mockup-spec 6), 272 wide:
//
//   a 28 pt strip   the segmented "Media N" | "Index", Search (⌘F), the
//                   List/Grid toggle (⌘⌥2) and Import (⌘I)
//   the media page  a grid on the well, 192 tall: 3 columns of 72 with
//                   72 x 38 thumbs (the poster, or a sound's waveform),
//                   duration badges in frames at the sequence's rate, an
//                   alpha checkerboard for media with alpha; or a list
//   the Index page  for the chosen media, its shots (the cuts as ranges),
//                   silences and loudest moments from the engine's media
//                   index; a row seeks the Source viewer
//   Roles           the rest of the band: one row per role of the program's
//                   tracks (tracks without one under their kind), whose box
//                   switches all of that role's tracks; Edit roles…
//
// Every row is engine data: the snapshot's media and tracks, media_status,
// media_info and media_index. Requests go out as signals; BinWiring sends them.

#include "test_friends.h"

#include <QHash>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QSize>
#include <QStringList>
#include <QPointer>
#include <QWidget>

class QAbstractButton;
class QLabel;
class QLineEdit;
class QListWidget;
class QListWidgetItem;
class QStackedWidget;
class QVBoxLayout;

class QAction;

namespace kerf {

class MediaArtwork;

namespace ui {
class CheckBox;
class IconButton;
} // namespace ui

class BinTabs;

class ProjectPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // The grid (mockup-spec 6): 3 columns of 72 tiles 12 apart, rows 8
    // apart, padded 8 16 0 on the well, 192 tall. An item is 56 tall: the
    // 72 x 38 thumb, 2 pt, then its name on a 16 pt line.
    static constexpr QSize kThumbnail{72, 38};
    static constexpr int kColumns = 3;
    static constexpr int kColumnGap = 12;
    static constexpr int kRowGap = 8;
    static constexpr int kItemHeight = 56;
    static constexpr int kNameTop = 2;
    static constexpr int kSidePadding = 16;
    static constexpr int kTopPadding = 8;
    static constexpr int kGridHeight = 192;
    // The Roles header and rows, 20 tall.
    static constexpr int kRoleRow = 20;
    // The list view's and the Index's rows.
    static constexpr int kListRow = 24;
    static constexpr int kIndexRow = 20;

    enum class Tab { Media, Index };

    // A role of the program's tracks, as the Roles rows show it.
    struct RoleGroup {
        QString key;         // the role as the engine names it, lower case ("dialogue"), or the kind
        QString name;        // "Dialogue"
        QStringList tracks;  // track ids, in timeline order
        bool enabled = true; // every track of the role is on
    };

    explicit ProjectPanel(MediaArtwork *artwork, QWidget *parent = nullptr);
    void setSources(const QJsonArray &sources);
    // A `media_status` event: {missing, pointers, external} media ids.
    void setMediaStatus(const QJsonObject &status);
    // Offline, Not downloaded, External, or empty.
    QString badge(const QString &mediaId) const { return m_badges.value(mediaId); }
    QLineEdit *search() const { return m_search; }

    // The program sequence's frame rate: duration badges count its frames.
    void setFps(double fps);
    // The program's tracks (timeline.tracks): the Roles rows.
    void setTracks(const QJsonArray &tracks);
    // A media_info event's `media` map {id: {codec, pixFmt, ...}}: media
    // whose pixel format carries alpha gets the checkerboard.
    void setMediaInfo(const QJsonObject &media);
    // A media_index event, kept by media; the Index shows the chosen one's.
    void setMediaIndex(const QJsonObject &index);
    // A media_index refusal for `media`.
    void setMediaIndexError(const QString &media, const QString &message);

    // The count after "Media" in the strip.
    int mediaCount() const;
    Tab tab() const { return m_tab; }
    void setTab(Tab tab);
    bool isListView() const { return m_listView; }
    void setListView(bool list);
    // The chosen media (the grid's current item), or empty.
    QString currentMedia() const;
    void setCurrentMedia(const QString &mediaId);
    // Shows the search field and puts the cursor in it (⌘F).
    void focusSearch();
    // The registry's bin.search (⌘F) and bin.toggleView (⌘⌥2): their keys
    // work while the bin has the focus (search) or is on screen (the view),
    // and the strip's tips name them. The window runs them (BinWiring).
    void setCommandActions(QAction *search, QAction *toggleView);
    // The strip's Import, which BinWiring keeps in step with file.import.
    QAbstractButton *importButton() const;
    QAbstractButton *editRolesButton() const { return m_editRoles; }

    QList<RoleGroup> roles() const { return m_roles; }
    // The rows Roles shows for `tracks` (timeline.tracks).
    static QList<RoleGroup> roleGroups(const QJsonArray &tracks);
    // A role's display name: "dialogue" -> "Dialogue"; a subtitle track's
    // missing role -> "Captions".
    static QString roleName(const QString &role, const QString &kind);
    // A duration badge (mockup "9s20f"): seconds and frames at `fps` under a
    // minute, minutes and seconds under an hour, else hours and minutes.
    static QString badgeText(double seconds, double fps);
    // Whether an FFmpeg pixel format carries alpha (yuva420p, rgba, ...).
    static bool pixelFormatHasAlpha(const QString &pixFmt);

signals:
    void openRequested(const QString &mediaId);
    // A right-click, on an item (its media id) or on the empty bin.
    void menuRequested(const QString &mediaId, const QPoint &globalPosition);
    void importRequested();
    // The Index page needs `mediaId`'s media_index.
    void indexWanted(const QString &mediaId);
    // An Index row: show `mediaId` in the Source viewer at `seconds`.
    void seekRequested(const QString &mediaId, double seconds);
    // A role's box: switch these tracks on or off.
    void rolesToggled(const QStringList &tracks, bool enabled);
    void editRolesRequested();
    void currentMediaChanged(const QString &mediaId);

protected:
    // Tiles are drawn at the screen's pixel ratio: moved to another screen,
    // they are drawn again.
    bool event(QEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;

private:
    void refresh();
    // Redraw every tile, for new artwork or appearance.
    void refreshTiles();
    QIcon tileFor(const QJsonObject &source) const;
    // Show only the media whose name holds `text`.
    void applyFilter(const QString &text);
    void rebuildRoles();
    void rebuildIndex();
    void requestIndex();
    void updateViewButton();
    QJsonObject sourceFor(const QString &mediaId) const;

    MediaArtwork *m_artwork = nullptr;
    QJsonArray m_sources;
    QHash<QString, QString> m_badges;
    QHash<QString, QJsonObject> m_info;
    QHash<QString, QJsonObject> m_indexes;
    QHash<QString, QString> m_indexErrors;
    // Media whose index was asked for and has not come back.
    QStringList m_indexAsked;
    QJsonArray m_tracks;
    QList<RoleGroup> m_roles;
    double m_fps = 24.0;
    Tab m_tab = Tab::Media;
    bool m_listView = false;

    BinTabs *m_tabs = nullptr;
    ui::IconButton *m_searchButton = nullptr;
    QPointer<QAction> m_searchAction;
    QPointer<QAction> m_viewAction;
    ui::IconButton *m_viewButton = nullptr;
    ui::IconButton *m_importButton = nullptr;
    QWidget *m_searchRow = nullptr;
    QLineEdit *m_search = nullptr;
    QStackedWidget *m_pages = nullptr;
    QWidget *m_well = nullptr;
    QListWidget *m_list = nullptr;
    QListWidget *m_index = nullptr;
    QLabel *m_indexEmpty = nullptr;
    QWidget *m_roleBox = nullptr;
    QVBoxLayout *m_roleRows = nullptr;
    QAbstractButton *m_editRoles = nullptr;
    QList<ui::CheckBox *> m_roleChecks;
};

} // namespace kerf
