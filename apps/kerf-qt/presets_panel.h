#pragma once

#include "test_friends.h"

#include <QJsonObject>
#include <QWidget>

class QAction;
class QLabel;
class QListWidget;
class QPushButton;
class QToolButton;

namespace kerf {

// The project's preset library: presets saved in presets/ travel with the
// project's repository, so collaborators share them. Apply binds a preset's
// media slots to this project's media; Save Current as Preset keeps the
// program sequence in the library.
class PresetsPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS

public:
    explicit PresetsPanel(QWidget *parent = nullptr);

    // A `preset_library` event; preset paths resolve against `projectDirectory`.
    void setLibrary(const QJsonObject &library, const QString &projectDirectory);
    // The registry's Save Current as Preset and Refresh Presets commands.
    void setActions(QAction *save, QAction *refresh);
    void setConnected(bool connected);

signals:
    // A preset's absolute path.
    void applyRequested(const QString &path);

private:
    void updateButtons();

    QListWidget *m_list = nullptr;
    QLabel *m_empty = nullptr;
    QPushButton *m_apply = nullptr;
    QToolButton *m_save = nullptr;
    QToolButton *m_refresh = nullptr;
    bool m_connected = false;
};

} // namespace kerf
