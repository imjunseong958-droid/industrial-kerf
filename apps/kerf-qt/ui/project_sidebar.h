#pragma once

// The sidebar: where the work is, and who is doing it. New conversation leads
// it, with a search over the chosen folder's conversations under it; then the
// recent project folders, the chosen one unfolded to its conversations by
// date; the account at its foot, which opens the way to Settings and
// everything else.
//
//   [+] New conversation              rows 30 tall, radius 6, UI 13/20
//   [search] Search conversations     28 tall
//   Projects                     [+]  meta in ink3; + is New Project, Open Folder
//   [folder] jeju-vlog      Working   on: on fill; hover: raised
//      Today                          meta in ink3, quiet
//      • Make a 60-second cut  Review a dot for what it is doing, then the
//      • Colour the beach take     4m state word or how long ago
//      Yesterday
//      • Captions for the vlog     1d  hover: [archive] [delete] in place of the time
//      > Archived · 2                 folded; unfolded, its rows are dimmed
//   [folder] atlas-launch
//   ...
//   (Y) You                       ⌃   the account, 48 tall
//       Kerf Cloud
//
// Nothing here is invented: the folders are the recent projects that still
// exist, the conversations and their times are what the bridge says they are,
// and the account is what the window says it is.
//
// Archive, rename and delete are asked for (the signals below) and shown at
// once; the next list from the bridge has the last word.

#include "components.h"
#include "test_friends.h"

#include <QDateTime>
#include <QList>
#include <QPointer>
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
    explicit SidebarRow(const QString &icon, const QString &text, int indent = 0, QWidget *parent = nullptr);
    // The word at the row's end, in `color`; empty for none.
    void setTrailing(const QString &text, const QColor &color);
    QString trailing() const { return m_trailing; }
    void setCurrent(bool current);
    bool isCurrent() const { return m_current; }
    // A 6 pt dot before the text, in place of an icon; an invalid colour for
    // none. A pulsing dot is drawn at the level setPulse() gives it.
    void setDot(const QColor &color, bool pulsing = false);
    // An icon drawn small where the dot would be: the archived section's chevron.
    void setDotIcon(const QString &icon);
    QColor dotColor() const { return m_dot; }
    bool isPulsing() const { return m_pulsing; }
    void setPulse(qreal level);
    // Drawn quieter: an archived conversation.
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // Buttons shown at the row's end while the pointer is over it, in place of
    // the trailing word. The row takes them as children.
    void setHoverActions(const QList<ui::IconButton *> &actions);
    QList<ui::IconButton *> hoverActions() const { return m_actions; }
    // Whether the hover actions show now.
    bool actionsShown() const;
    // The text is being edited over the row: neither it nor the actions are drawn.
    void setEditing(bool editing);
    // Where the text starts.
    int textLeft() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void placeActions();
    QRect dotRect() const;

    QString m_icon;
    QString m_dotIcon;
    QString m_trailing;
    QColor m_trailingColor;
    QColor m_dot;
    qreal m_pulse = 1.0;
    QList<ui::IconButton *> m_actions;
    int m_indent = 0;
    bool m_current = false;
    bool m_pulsing = false;
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

    // The folders, most recent first; those gone from disk are left out, and
    // `current` is always listed.
    void setProjects(const QStringList &paths, const QString &current);
    QStringList projects() const { return m_projects; }
    QString current() const { return m_current; }
    void setConversations(const QList<Conversation> &conversations, const QString &currentId);
    QList<Conversation> conversations() const { return m_conversations; }
    // Whether the chosen folder unfolds (the start window lists folders alone).
    void setConversationsShown(bool shown);
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
    // The date labels over the chosen folder's conversations, in order.
    QStringList sectionLabels() const;
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

signals:
    void projectChosen(const QString &path);
    void conversationChosen(const QString &id);
    void newConversationRequested();
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
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    // `arrivals` fades in the rows that were not here before; filtering and
    // folding show what was there all along.
    void rebuild(bool arrivals = true, bool unfolding = false);
    SidebarRow *makeConversationRow(const Conversation &conversation, bool dimmed);
    void showConversationMenu(const QString &id, const QPoint &globalPos, bool fromKeyboard);
    void refreshTimes();
    void updatePulse();
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
    QString m_query;
    // Stands in for the clock in tests; null follows it.
    QDateTime m_fixedNow;
    QDate m_builtOn;
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
    QString m_menuFor;
    SidebarAccount *m_account = nullptr;
    QList<SidebarRow *> m_rows;
    QStringList m_rowKeys;
    // What else is in the list: the date labels and the no-match note.
    QList<QWidget *> m_extras;
    QStringList m_sectionTexts;
    QVariantAnimation m_pulse;
    int m_pulseStep = -1;
    QTimer m_clock;
    // The title being edited, and over which conversation.
    QPointer<QLineEdit> m_renameField;
    QString m_renaming;
};

} // namespace kerf
