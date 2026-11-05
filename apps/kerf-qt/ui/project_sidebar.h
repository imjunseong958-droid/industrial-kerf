#pragma once

// The sidebar: where the work is, as a list of folders. New Project and Open
// Folder lead it; then the recent project folders, the chosen one unfolded to
// its conversations (the review log's threads) under New conversation.
//
//   [+] New Project…                  rows 28 tall, radius 6, UI 12/16
//   [folder] Open Folder…             on: on fill; hover: raised
//   Projects                          meta in ink3
//   [folder] jeju-vlog      Working
//        [+] New conversation
//        Make a 60-second cut  Review
//   [folder] atlas-launch
//
// Nothing here is invented: the folders are the recent projects that still
// exist, and a conversation's title is its first comment's first line.

#include "components.h"
#include "test_friends.h"

#include <QList>
#include <QString>
#include <QStringList>
#include <QWidget>

class QLabel;
class QVBoxLayout;

namespace kerf {

namespace ui {
class FadeScrollArea;
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

class ProjectSidebar final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class State { Idle, Review, Settled };
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
    SidebarRow *newConversationRow() const { return m_newConversation; }
    SidebarRow *newProjectRow() const { return m_newProject; }
    SidebarRow *openFolderRow() const { return m_openFolder; }

signals:
    void projectChosen(const QString &path);
    void conversationChosen(const QString &id);
    void newConversationRequested();
    void newProjectRequested();
    void openFolderRequested();

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
    SidebarRow *m_newProject = nullptr;
    SidebarRow *m_openFolder = nullptr;
    SidebarRow *m_newConversation = nullptr;
    QList<SidebarRow *> m_rows;
    QStringList m_rowKeys;
};

} // namespace kerf
