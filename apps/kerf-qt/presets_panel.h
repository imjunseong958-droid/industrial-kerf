#pragma once

// The browser's Presets page (mockup-spec 8): the project's preset library.
// Presets saved in presets/ travel with the project's repository, so
// collaborators share them. Under the "Shared presets · N items" row, each
// preset is a 44 pt row with its name and meta (clips · slots · date); the
// footer has Save current as preset… and Apply…, which binds a preset's
// media slots to this project's media.

#include "test_friends.h"

#include <QJsonObject>
#include <QWidget>

class QAction;
class QLabel;
class QListWidget;

namespace kerf {

namespace ui {
class Button;
class IconButton;
} // namespace ui

class PresetsPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS

public:
    static constexpr int kRowHeight = 44;

    explicit PresetsPanel(QWidget *parent = nullptr);

    // A `preset_library` event; preset paths resolve against `projectDirectory`.
    void setLibrary(const QJsonObject &library, const QString &projectDirectory);
    // The registry's Save Current as Preset and Refresh Presets commands.
    void setActions(QAction *save, QAction *refresh);
    void setConnected(bool connected);
    // A preset row's meta line: "3 clips · 2 slots · 28 Oct", or why it
    // cannot be read.
    static QString metaLine(const QJsonObject &preset);

signals:
    // A preset's absolute path.
    void applyRequested(const QString &path);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void updateButtons();
    void followActions();

    QListWidget *m_list = nullptr;
    QLabel *m_empty = nullptr;
    QLabel *m_count = nullptr;
    ui::Button *m_apply = nullptr;
    ui::Button *m_save = nullptr;
    ui::IconButton *m_refresh = nullptr;
    QAction *m_saveAction = nullptr;
    QAction *m_refreshAction = nullptr;
    bool m_connected = false;
};

} // namespace kerf
