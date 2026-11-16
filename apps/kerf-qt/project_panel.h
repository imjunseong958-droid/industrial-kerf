#pragma once

// The Media sheet (mockup-spec 6), 272 wide beside the conversation:
//
//   a 28 pt strip   the segmented "Media N" | "Index", Search (⌘F), the
//                   List/Grid toggle (⌘⌥2) and Import (⌘I)
//   the media page  a grid on the well, as tall as the sheet leaves it: 2
//                   columns of 112 with 112 x 63 thumbs (the poster, or a
//                   sound's waveform), duration badges in frames at the
//                   sequence's rate, an alpha checkerboard for media with
//                   alpha; under each its whole name and what it is doing
//                   ("Made · shot · take 1 of 1 · on V1", "Making take 2 ·
//                   0:41" over amber stripes moving on its thumb, "Wants
//                   making · …", "Offline"); or a list
//   the Index page  for the chosen media, its shots (the cuts as ranges),
//                   silences and loudest moments from the engine's media
//                   index
//   Roles           under the page: one row per role of the program's
//                   tracks (tracks without one under their kind), whose box
//                   switches all of that role's tracks; Edit roles…
//
// Every row is engine data: the snapshot's media and tracks, media_status,
// media_info and media_index. Requests go out as signals; BinWiring sends them.

#include "test_friends.h"
#include "ui/activity.h"

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
class QHBoxLayout;
class QLabel;
class QLineEdit;
class QListWidget;
class QListWidgetItem;
class QStackedWidget;
class QTimer;
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
    // The grid: 2 columns of 112 tiles 12 apart, rows 12 apart, padded 12 16 0
    // on the well, which has what Roles leaves of the sheet. An item is 101
    // tall: the 112 x 63 thumb, 6 pt, then its name and what it is doing, each
    // on a 16 pt line.
    static constexpr QSize kThumbnail{112, 63};
    static constexpr int kColumns = 2;
    static constexpr int kColumnGap = 12;
    static constexpr int kRowGap = 12;
    static constexpr int kNameTop = 6;
    static constexpr int kLine = 16;
    static constexpr int kItemHeight = 63 + kNameTop + 2 * kLine;
    static constexpr int kSidePadding = 16;
    static constexpr int kTopPadding = 12;
    // The page at least: a row of tiles and the start of the next.
    static constexpr int kGridLeast = 160;
    // The Roles header and rows, 20 tall; Roles is as tall as they are, to this.
    static constexpr int kRoleRow = 20;
    static constexpr int kRolesMost = 164;
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
    // What each media is doing (MainWindow::mediaActivity), and the shots a
    // cast event said want making, each with why.
    void setMediaActivity(const QHash<QString, activity::MediaActivity> &media);
    void setWantsMaking(const QHash<QString, QString> &wants);
    activity::MediaActivity mediaActivity(const QString &mediaId) const { return m_activity.value(mediaId); }
    // The line under a tile's name: what `mediaId` is doing now.
    QString status(const QString &mediaId) const;
    // That line for `source`: being made ("Making take 2 · 0:41"), queued, not
    // made yet or offline, wanting making again (`wants`, why), else what it is,
    // which take and where it is used ("Made · shot · take 1 of 2 · on V1",
    // "Video · on V1 V2").
    static QString statusText(const QJsonObject &source, const activity::MediaActivity &activity, const QString &badge,
                              const QString &wants, const QStringList &usedOn, qint64 now);
    // Whether the stripes over a tile being made draw frames.
    bool makingTicks() const;
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

    // A control at the end of the strip (the sheet's close button).
    void addToStrip(QWidget *widget);

signals:
    void openRequested(const QString &mediaId);
    // A right-click, on an item (its media id) or on the empty bin.
    void menuRequested(const QString &mediaId, const QPoint &globalPosition);
    void importRequested();
    // The Index page needs `mediaId`'s media_index.
    void indexWanted(const QString &mediaId);
    // An Index row was chosen: `mediaId` at `seconds`.
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
    QHBoxLayout *m_stripLayout = nullptr;
    void refresh();
    // Redraw every tile, for new artwork or appearance.
    void refreshTiles();
    QIcon tileFor(const QJsonObject &source) const;
    void followMakingTicks();
    // The tracks each media's clips are on, as their badges say them ("V1").
    QHash<QString, QStringList> m_usedOn;
    QHash<QString, activity::MediaActivity> m_activity;
    QHash<QString, QString> m_wants;
    QTimer *m_makingTicker = nullptr;
    QWidget *m_rolesArea = nullptr;
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
