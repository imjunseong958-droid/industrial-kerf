#pragma once

// The sidebar: where the work is, and who is doing it. New conversation leads
// it, with a search over the conversations under it; then the recent project
// folders, each one unfolding to its own conversations, latest first; the
// account at its foot, which opens the way to Settings and everything else.
//
//   [+] New conversation              rows 32 apart, radius 6, UI 13/20
//   [search] Search conversations     30 tall
//   Projects                     [+]  meta in ink3; + is New Project, Open Folder
//   v [folder] jeju-vlog    [+] [⋯]   the open folder's name in ink, medium;
//   |   Make a 60-second cut of th…   hover: + (a conversation in it) and its menu
//   |   Colour the beach take      ●  titles under the folder's icon, with the room
//   |   Captions for the vlog         to themselves; a light passes over a title
//   |   Show 3 more                   Kerf works on, and a dot at the end says it
//   |   > Archived · 2                is done, waits for you, or failed; hover: [⋯]
//   > [folder] atlas-launch           folded: pressing its chevron unfolds it,
//   ...                               pressing its name opens it
//   (Y) You                       ⌃   the account, 52 tall
//       Kerf Cloud
//
// Nothing here is invented: the folders are the recent projects that still
// exist, the open folder's conversations and their times are what the bridge
// says they are, another folder's are what it keeps beside its project
// (.kerf/conversations), and the account is what the window says it is.
//
// Archive, rename and delete are asked for (the signals below) and shown at
// once; the next list from the bridge has the last word. They are the open
// folder's alone: another folder's conversation is opened, with its project.

#include "components.h"
#include "test_friends.h"

#include <QDateTime>
#include <QHash>
#include <QList>
#include <QPointer>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QVariantAnimation>
#include <QWidget>

class QLabel;
class QLineEdit;
class QMenu;
class QVBoxLayout;

namespace kerf {

namespace ui {
class FadeScrollArea;
class IconButton;
} // namespace ui

class SidebarRow final : public ui::KitButton {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // Whether a row unfolds to others, and which way its chevron points.
    enum class Chevron { None, Folded, Unfolded };
    explicit SidebarRow(const QString &icon, const QString &text, int indent = 0, QWidget *parent = nullptr);
    // A chevron before the icon: pressed, it says so (chevronPressed) and the
    // row is not clicked.
    void setChevron(Chevron chevron);
    Chevron chevron() const { return m_chevron; }
    // Where the chevron takes a press; empty with none.
    QRect chevronRect() const;
    // The text in medium, and in ink at rest: a folder's name.
    void setStrong(bool strong);
    bool isStrong() const { return m_strong; }
    // A hairline down the row, under its folder's chevron: it belongs to the
    // folder over it.
    void setGuide(bool guide);
    bool hasGuide() const { return m_guide; }
    void setCurrent(bool current);
    bool isCurrent() const { return m_current; }
    // A dot at the row's end, in `color`: something there to look at. An
    // invalid colour for none.
    void setBadge(const QColor &color);
    QColor badge() const { return m_badge; }
    // A light passes over the title while Kerf works on it, at `phase` (0 to 1)
    // of its pass; still, the title is drawn in ink.
    void setShimmer(bool shimmer);
    bool isShimmering() const { return m_shimmer; }
    void setShimmerPhase(qreal phase);
    // An icon drawn small before the text, in place of an icon: the archived section's chevron.
    void setDotIcon(const QString &icon);
    // Drawn quieter: an archived conversation.
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // Buttons shown at the row's end while the pointer is over it, in place of
    // the dot. The row takes them as children.
    void setHoverActions(const QList<ui::IconButton *> &actions);
    QList<ui::IconButton *> hoverActions() const { return m_actions; }
    // Whether the hover actions show now.
    bool actionsShown() const;
    // The text is being edited over the row: neither it nor the actions are drawn.
    void setEditing(bool editing);
    // Where the text starts.
    int textLeft() const;
    QSize sizeHint() const override;

signals:
    void chevronPressed();

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    void placeActions();
    // Where the title is drawn, to the badge or the actions.
    QRect textRect() const;

    QString m_icon;
    QString m_dotIcon;
    QColor m_badge;
    qreal m_phase = 0.0;
    QList<ui::IconButton *> m_actions;
    int m_indent = 0;
    Chevron m_chevron = Chevron::None;
    bool m_onChevron = false;
    bool m_strong = false;
    bool m_guide = false;
    bool m_current = false;
    bool m_shimmer = false;
    bool m_dimmed = false;
    bool m_editing = false;
};

// The account at the sidebar's foot: an initial in a disc, a name over what it
// is signed in to, and the chevrons of something that opens upward.
class SidebarAccount final : public ui::KitButton {
    Q_OBJECT
public:
    explicit SidebarAccount(QWidget *parent = nullptr);
    void setAccount(const QString &title, const QString &subtitle);
    QString title() const { return m_title; }
    QString subtitle() const { return m_subtitle; }
    // Held on while its menu is open.
    void setOpen(bool open);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_title;
    QString m_subtitle;
    bool m_open = false;
};

class ProjectSidebar final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // What Kerf is doing in a conversation: nothing, working on a turn, making
    // a cut or a render, waiting for you to look, done, or stopped on an error.
    enum class State { Idle, Working, Review, Settled, Making, Failed };
    struct Conversation {
        QString id;
        QString title;
        State state = State::Idle;
        bool archived = false;
        // When it last changed; a conversation with none is listed first, with no time.
        QDateTime updated;
        // How many messages it holds.
        int count = 0;
        bool operator==(const Conversation &other) const {
            return id == other.id && title == other.title && state == other.state && archived == other.archived &&
                   updated == other.updated && count == other.count;
        }
    };

    explicit ProjectSidebar(QWidget *parent = nullptr);

    // How many of a folder's conversations show before Show more.
    static constexpr int kShownAtFirst = 6;

    // The folders, most recent first; those gone from disk are left out, and
    // `current` is always listed.
    void setProjects(const QStringList &paths, const QString &current);
    QStringList projects() const { return m_projects; }
    QString current() const { return m_current; }
    // The open folder's conversations, as the bridge lists them.
    void setConversations(const QList<Conversation> &conversations, const QString &currentId);
    QList<Conversation> conversations() const { return m_conversations; }
    // A folder's conversations as the sidebar lists them: the open folder's
    // from the bridge, another's from what it keeps beside its project.
    QList<Conversation> conversationsOf(const QString &path) const;
    // The conversations a project folder keeps (.kerf/conversations), read
    // from its files: their titles, times and whether they are archived.
    static QList<Conversation> readConversations(const QString &project);
    // Whether the folders unfold (the start window lists folders alone).
    void setConversationsShown(bool shown);
    // A folder folded shows its name alone. The open folder starts unfolded and
    // the others folded; what is chosen here is kept (sidebar/folded).
    bool isFolded(const QString &path) const;
    void setFolded(const QString &path, bool folded);
    // Kerf is working in the chosen folder.
    void setWorking(bool working);
    // Room over the first row, for the window buttons of a window with no toolbar.
    void setTopInset(int inset);

    QList<SidebarRow *> rows() const { return m_rows; }
    SidebarRow *projectRow(const QString &path) const;
    SidebarRow *conversationRow(const QString &id) const;
    // New conversation, while a folder's conversations show.
    SidebarRow *newConversationRow() const;
    // The search over the chosen folder's conversations.
    QLineEdit *searchField() const { return m_search; }
    // The row that folds and unfolds the archived conversations; null when there are none.
    SidebarRow *archivedRow() const;
    bool archivedOpen() const { return m_archivedOpen; }
    void setArchivedOpen(bool open);
    // The row that shows the rest of a folder's conversations; null when all show.
    SidebarRow *moreRow(const QString &path) const;
    // What stands in for a folder's rows when it has none: "No conversations yet".
    QStringList notes() const { return m_noteTexts; }
    // A folder's menu (New Conversation, Open, Show in Folder, Copy Path,
    // Remove from Sidebar), made ready for `path`.
    QMenu *projectMenu(const QString &path);
    // The + beside Projects, and the menu it opens: New Project, Open Folder.
    ui::IconButton *addButton() const { return m_add; }
    QMenu *addMenu() const { return m_addMenu; }
    // A conversation's menu (Rename…, Archive or Unarchive, Delete…), made
    // ready for `id`; a right-click or the menu key opens it over its row.
    QMenu *conversationMenu(const QString &id);
    // The account at the foot; hidden until it is given something to say.
    SidebarAccount *accountRow() const { return m_account; }
    void setAccount(const QString &title, const QString &subtitle);

    // Asks to archive (or unarchive) a conversation.
    void archive(const QString &id, bool archived);
    // Asks whether to delete a conversation, and asks for it if so.
    void confirmDelete(const QString &id);
    // Edits a conversation's title over its row: Enter keeps it, Esc does not.
    void beginRename(const QString &id);

    // "now", "4m", "2h", "3d", then a short date: how long before `now` `when` was.
    static QString relativeTime(const QDateTime &when, const QDateTime &now);
    // Conversations that finished while you were elsewhere: their dot stays
    // until you look at them.
    QSet<QString> unseen() const { return m_unseen; }

signals:
    void projectChosen(const QString &path);
    void conversationChosen(const QString &id);
    // A conversation of a folder that is not the open one: its project opens, on it.
    void conversationChosenIn(const QString &project, const QString &id);
    void newConversationRequested();
    // A folder's menu: take it off the list (nothing on disk is touched), or
    // show it in the system's file manager.
    void projectRemoveRequested(const QString &path);
    void projectRevealRequested(const QString &path);
    void newProjectRequested();
    void openFolderRequested();
    // The account was pressed: the window opens its menu over it.
    void accountRequested();
    void archiveRequested(const QString &id, bool archived);
    // Only once the deletion has been confirmed.
    void deleteRequested(const QString &id);
    void renameRequested(const QString &id, const QString &title);

protected:
    void paintEvent(QPaintEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;
    void changeEvent(QEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    // `arrivals` fades in the rows that were not here before; filtering and
    // folding show what was there all along.
    void rebuild(bool arrivals = true, bool unfolding = false);
    // `project` is the folder it belongs to: the open one's rows can be
    // archived, renamed and deleted; another's only opened.
    SidebarRow *makeConversationRow(const Conversation &conversation, bool dimmed, const QString &project);
    SidebarRow *makeProjectRow(const QString &path);
    void showConversationMenu(const QString &id, const QPoint &globalPos, bool fromKeyboard);
    // Another folder's conversations, read again from its files.
    void readOthers();
    void refreshTimes();
    void updateShimmer();
    // The dot at a conversation's end, and the words that say it.
    QColor badgeOf(const Conversation &conversation) const;
    QString stateOf(const Conversation &conversation) const;
    // Whether the window is where you are looking: what finishes then is seen.
    bool watching() const;
    // The open conversation, looked at, is seen.
    void markSeen();
    void finishRename(bool keep);
    QDateTime now() const;
    int indexOf(const QString &id) const;

    QStringList m_projects;
    QString m_current;
    QList<Conversation> m_conversations;
    QString m_currentConversation;
    bool m_conversationsShown = true;
    bool m_working = false;
    bool m_archivedOpen = false;
    // Folded or not, as chosen (and kept); a folder not here has its default.
    QHash<QString, bool> m_folded;
    // The folders whose conversations all show, past the first few.
    QSet<QString> m_showAll;
    // What the folders other than the open one keep, as last read.
    QHash<QString, QList<Conversation>> m_others;
    QString m_query;
    // Stands in for the clock in tests; null follows it.
    QDateTime m_fixedNow;
    QVBoxLayout *m_outer = nullptr;
    QVBoxLayout *m_list = nullptr;
    QWidget *m_top = nullptr;
    QLabel *m_heading = nullptr;
    ui::FadeScrollArea *m_scroll = nullptr;
    SidebarRow *m_newConversation = nullptr;
    QLineEdit *m_search = nullptr;
    ui::IconButton *m_add = nullptr;
    QMenu *m_addMenu = nullptr;
    QMenu *m_conversationMenu = nullptr;
    QMenu *m_projectMenu = nullptr;
    QString m_menuFor;
    QString m_projectMenuFor;
    SidebarAccount *m_account = nullptr;
    QList<SidebarRow *> m_rows;
    QStringList m_rowKeys;
    // What else is in the list: the notes that stand in for rows.
    QList<QWidget *> m_extras;
    QStringList m_noteTexts;
    QSet<QString> m_unseen;
    QVariantAnimation m_shimmer;
    int m_shimmerStep = -1;
    QTimer m_clock;
    // The title being edited, and over which conversation.
    QPointer<QLineEdit> m_renameField;
    QString m_renaming;
};

} // namespace kerf
