#pragma once

#include "editor_theme.h"
#include "launcher_style.h"

#include <QDialog>
#include <QFocusFrame>
#include <QHash>
#include <QJsonObject>
#include <QList>
#include <QPointer>
#include <QStringList>

#include <functional>

class QCheckBox;
class QComboBox;
class QDoubleSpinBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QProgressBar;
class QPushButton;
class QStackedWidget;
class QTimer;
class QToolButton;
class QVBoxLayout;

namespace kerf {

// The window Kerf opens with when it is given no project, and File ▸ New
// Project…: make a project folder from a template or from a shared preset,
// clone a shared project, or open a recent or existing one, as edit suites
// do in their Choose a Project window. Accepting it leaves the project's
// folder in chosenPath().
class ProjectLauncher final : public QDialog {
    Q_OBJECT
    friend class EditorTest;

public:
    enum Page { NewProject, Recent, FromPreset, Clone };
    // System follows macOS's appearance as it changes; KERF_COLOR_SCHEME=light
    // or dark picks one for the whole run. A launcher starts in the editor's
    // scheme (editorColorScheme()).
    using ColorScheme = kerf::ColorScheme;

    // `kerfdBinary` makes the first edit of a project made from a preset.
    explicit ProjectLauncher(const QString &kerfdBinary = QString(), QWidget *parent = nullptr);
    ~ProjectLauncher() override;

    QString chosenPath() const { return m_chosen; }
    void showPage(Page page);
    void setColorScheme(ColorScheme scheme);
    ColorScheme colorScheme() const { return m_scheme; }
    const LauncherTheme &theme() const { return m_theme; }

    // Recent projects (QSettings `projects/recent`), most recent first.
    static QStringList recentProjects();
    static void rememberProject(const QString &path);
    static void clearRecentProjects();
    static constexpr int kRecentLimit = 12;

protected:
    void changeEvent(QEvent *event) override;
    void showEvent(QShowEvent *event) override;

private:
    // Restyle for the current appearance and accent; does nothing when
    // neither changed unless `force`.
    void applyTheme(bool force = false);
    void setDetailsOpen(bool open);
    // Fit the sidebar's top and the title band to the title bar macOS
    // draws over them.
    void fitTitleBar();
    // Ring the focused field the way macOS does.
    void trackFocus(QWidget *now);

    QWidget *buildTemplates();
    QWidget *buildRecent();
    QWidget *buildPresetPage();
    QWidget *buildClone();
    QWidget *buildDetails();
    QWidget *buildFooter();

    void reloadRecent();
    void reloadPresetChoices();
    // Read a preset's slots, then show a row for each.
    void loadPreset(const QString &path);
    void showPreset(const QString &path, const QJsonObject &preset);
    void chooseSlotFile(const QString &slot);
    void setSlotFile(const QString &slot, const QString &file);
    // Ask which slots the chosen files can fill.
    void refit();
    // What `{type: tools}` found: git, Git LFS, and the GitHub CLI.
    void setTools(const QJsonObject &tools);

    void openExisting();
    void choose();
    // Why Choose cannot run on this page, or empty when it can.
    QString problem() const;
    QString detailsProblem() const;
    void updateChoose();
    void updateDetailsSummary();
    QString templateId() const;
    QJsonObject createRequest() const;
    // The folder a project called `name` gets in the chosen location.
    QString projectFolder() const;
    QString cloneFolder() const;

    // Start a kerf_task_* task and pass each status it reports to `update`
    // until it finishes. Test builds only record the request.
    void runTask(const QJsonObject &request, std::function<void(const QJsonObject &)> update);
    // A create or clone task's status.
    void handleTaskStatus(const QJsonObject &status);
    void setBusy(bool busy, const QString &message = QString());
    void showFailure(const QString &message);

    QString m_kerfd;
    QString m_chosen;
    ColorScheme m_scheme = ColorScheme::System;
    LauncherTheme m_theme;
    bool m_themed = false;
    bool m_theming = false;
    QPointer<QFocusFrame> m_focusRing;
    QJsonObject m_tools;
    bool m_busy = false;
    // Every task asked for, in order; tests read it.
    QList<QJsonObject> m_requests;
    QHash<QTimer *, void *> m_tasks;
    // kerf_project_inspect and kerf_preset_inspect; tests replace them.
    std::function<QJsonObject(const QJsonObject &)> m_inspectProject;
    std::function<QJsonObject(const QJsonObject &)> m_inspectPreset;

    QListWidget *m_sidebar = nullptr;
    QWidget *m_sidebarTop = nullptr;
    QLabel *m_title = nullptr;
    QVBoxLayout *m_pagesLayout = nullptr;
    QStackedWidget *m_pages = nullptr;
    QWidget *m_headingBox = nullptr;
    QLabel *m_heading = nullptr;
    QLabel *m_subheading = nullptr;
    QListWidget *m_templates = nullptr;
    QListWidget *m_recent = nullptr;
    QListWidget *m_presetChoices = nullptr;
    QLabel *m_presetTitle = nullptr;
    QWidget *m_presetSlots = nullptr;
    QVBoxLayout *m_presetSlotsLayout = nullptr;
    QString m_presetPath;
    QJsonObject m_preset;
    QStringList m_slotOrder;
    QHash<QString, QString> m_slotFiles;
    QHash<QString, bool> m_slotFits;
    QHash<QString, QLabel *> m_slotStatus;
    // Where each slot starts in its file, in seconds.
    QHash<QString, QDoubleSpinBox *> m_slotStarts;
    QHash<QString, QLineEdit *> m_presetTexts;
    QLineEdit *m_cloneUrl = nullptr;
    QLineEdit *m_cloneLocation = nullptr;
    QLabel *m_cloneDestination = nullptr;
    QWidget *m_details = nullptr;
    QWidget *m_detailsRow = nullptr;
    QToolButton *m_detailsToggle = nullptr;
    QLabel *m_detailsSummary = nullptr;
    QWidget *m_detailsBody = nullptr;
    QLineEdit *m_name = nullptr;
    QLineEdit *m_location = nullptr;
    QComboBox *m_rate = nullptr;
    QCheckBox *m_git = nullptr;
    QCheckBox *m_github = nullptr;
    QLabel *m_gitHint = nullptr;
    QLineEdit *m_remote = nullptr;
    QLabel *m_destination = nullptr;
    QLabel *m_problem = nullptr;
    QProgressBar *m_progress = nullptr;
    QPushButton *m_openExisting = nullptr;
    QPushButton *m_cancel = nullptr;
    QPushButton *m_choose = nullptr;
};

} // namespace kerf
