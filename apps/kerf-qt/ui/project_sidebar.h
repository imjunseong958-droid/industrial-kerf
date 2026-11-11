#pragma once

// The sidebar: where the work is, and who is doing it. New conversation leads
// it; then the recent project folders, the chosen one unfolded to its
// conversations (the review log's threads); the account at its foot, which
// opens the way to Settings and everything else.
//
//   [+] New conversation              rows 30 tall, radius 6, UI 13/20
//   Projects                     [+]  meta in ink3; + is New Project, Open Folder
//   [folder] jeju-vlog      Working   on: on fill; hover: raised
//        Make a 60-second cut  Review
//   [folder] atlas-launch
//   ...
//   (Y) You                       ⌃   the account, 48 tall
//       Kerf Cloud
//
// Nothing here is invented: the folders are the recent projects that still
// exist, a conversation's title is its first comment's first line, and the
// account is what the window says it is.

#include "components.h"
#include "test_friends.h"

#include <QList>
#include <QString>
#include <QStringList>
#include <QWidget>

class QLabel;
class QMenu;
class QVBoxLayout;

namespace kerf {

namespace ui {
class FadeScrollArea;
class IconButton;
} // namespace ui

class SidebarRow final : public ui::KitButton {
    Q_OBJECT
public:
    explicit SidebarRow(const QString &icon, const QString &text, int indent = 0, QWidget *parent = nullptr);
    // The word at the row's end, in `color`; empty for none.
    void setTrailing(const QString &text, const QColor &color);
    QString trailing() const { return m_trailing; }
    void setCurrent(bool current);
    bool isCurrent() const { return m_current; }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_icon;
    QString m_trailing;
    QColor m_trailingColor;
    int m_indent = 0;
    bool m_current = false;
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
    enum class State { Idle, Working, Review, Settled };
    struct Conversation {
        QString id;
        QString title;
        State state = State::Idle;
        bool operator==(const Conversation &other) const {
            return id == other.id && title == other.title && state == other.state;
        }
    };

    explicit ProjectSidebar(QWidget *parent = nullptr);

    // The folders, most recent first; those gone from disk are left out, and
    // `current` is always listed.
    void setProjects(const QStringList &paths, const QString &current);
    QStringList projects() const { return m_projects; }
    QString current() const { return m_current; }
    void setConversations(const QList<Conversation> &conversations, const QString &currentId);
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
    // The + beside Projects, and the menu it opens: New Project, Open Folder.
    ui::IconButton *addButton() const { return m_add; }
    QMenu *addMenu() const { return m_addMenu; }
    // The account at the foot; hidden until it is given something to say.
    SidebarAccount *accountRow() const { return m_account; }
    void setAccount(const QString &title, const QString &subtitle);

signals:
    void projectChosen(const QString &path);
    void conversationChosen(const QString &id);
    void newConversationRequested();
    void newProjectRequested();
    void openFolderRequested();
    // The account was pressed: the window opens its menu over it.
    void accountRequested();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void rebuild();

    QStringList m_projects;
    QString m_current;
    QList<Conversation> m_conversations;
    QString m_currentConversation;
    bool m_conversationsShown = true;
    bool m_working = false;
    QVBoxLayout *m_outer = nullptr;
    QVBoxLayout *m_list = nullptr;
    QWidget *m_top = nullptr;
    QLabel *m_heading = nullptr;
    ui::FadeScrollArea *m_scroll = nullptr;
    SidebarRow *m_newConversation = nullptr;
    ui::IconButton *m_add = nullptr;
    QMenu *m_addMenu = nullptr;
    SidebarAccount *m_account = nullptr;
    QList<SidebarRow *> m_rows;
    QStringList m_rowKeys;
};

} // namespace kerf
