#pragma once

// The window Kerf opens with when it is given no project: the folders at the
// left and one question in the middle. Writing a request and sending it opens
// the chosen folder's editor with that request on its way to Kerf; a folder
// with no project.kerf becomes a project, with its media imported. New
// Project… is the Choose a Project window, for templates, presets and clones.

#include "test_friends.h"

#include <QDialog>
#include <QList>
#include <QPixmap>
#include <QRect>
#include <QString>
#include <QStringList>

#include <functional>

namespace kerf {

class Composer;
class ProjectSidebar;

namespace ui {
class KitButton;
class GhostButton;
} // namespace ui

class HomeWindow final : public QDialog {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit HomeWindow(const QString &kerfdBinary = QString(), QWidget *parent = nullptr);

    // Once accepted: the folder, and the request to send it (empty to just open).
    QString chosenPath() const { return m_folder; }
    QString request() const { return m_request; }
    // The folder is not a project yet, so its media goes in first.
    bool startsProject() const { return m_startsProject; }

    void setFolder(const QString &path);
    // What the window looked like as it was accepted, and where its request box
    // was, for the editor that takes its place.
    QPixmap lastLook() const { return m_lastLook; }
    QRect composerRect() const;
    static QStringList suggestions();

signals:
    // A folder is chosen, with what to do in it: result() is Accepted and the
    // window stays up, for the editor to open over it.
    void chosen();

protected:
    void showEvent(QShowEvent *event) override;
    void paintEvent(QPaintEvent *event) override;

private:
    void chooseFolder();
    void newProject();
    void send();
    void finish(const QString &request);
    void updateControls();
    void showFolderMenu();

    void enter();
    QPixmap m_lastLook;
    bool m_entered = false;
    QWidget *m_heading = nullptr;
    QWidget *m_ideaRow = nullptr;
    QString m_kerfd;
    QString m_folder;
    QString m_request;
    bool m_startsProject = false;
    ProjectSidebar *m_sidebar = nullptr;
    Composer *m_composer = nullptr;
    ui::GhostButton *m_folderChip = nullptr;
    ui::GhostButton *m_openOnly = nullptr;
    QList<ui::KitButton *> m_ideas;
    // The folder picker; tests replace it.
    std::function<QString()> m_pickFolder;
};

} // namespace kerf
