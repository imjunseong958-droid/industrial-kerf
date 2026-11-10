// Tests for the project launcher.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
#include "home_window.h"
#include "project_launcher.h"
#include "ui/composer.h"
#include "ui/project_sidebar.h"
#include "project_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"

#include <QAbstractItemView>
#include <QAccessible>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMenuBar>
#include <QMimeData>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QScreen>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QWheelEvent>
#include <QWidget>


#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

class LauncherTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void projectLauncherValidatesWithoutStartingTasks() {
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
        QTemporaryDir root;
        const QString location = QDir(root.path()).filePath(QStringLiteral("Projects"));
        const QString taken = QDir(location).filePath(QStringLiteral("Taken"));
        const QString old = QDir(root.path()).filePath(QStringLiteral("Old Cut"));
        QVERIFY(QDir().mkpath(QDir(taken).filePath(QStringLiteral("media"))));
        QVERIFY(QDir().mkpath(old));

        // Recent projects: most recent first, without repeats, at most twelve.
        for (int i = 0; i < 20; ++i) ProjectLauncher::rememberProject(QDir(root.path()).filePath(QStringLiteral("p%1").arg(i)));
        QCOMPARE(ProjectLauncher::recentProjects().size(), ProjectLauncher::kRecentLimit);
        QCOMPARE(ProjectLauncher::recentProjects().first(), QDir(root.path()).filePath(QStringLiteral("p19")));
        ProjectLauncher::clearRecentProjects();
        ProjectLauncher::rememberProject(old);
        ProjectLauncher::rememberProject(taken);
        ProjectLauncher::rememberProject(old);
        QCOMPARE(ProjectLauncher::recentProjects(), (QStringList{old, taken}));

        ProjectLauncher launcher(QStringLiteral("/opt/kerf/bin/kerfd"));
        // It asks for the tools; a test build starts no kerf_task_* task.
        QCOMPARE(launcher.m_requests, (QList<QJsonObject>{QJsonObject{{"type", "tools"}}}));
        QVERIFY(launcher.m_tasks.isEmpty());
        // Without an inspection the folder name stands in.
        QVERIFY(launcher.m_recent->item(0)->text().startsWith(QStringLiteral("Old Cut\n")));
        QCOMPARE(launcher.m_sidebar->count(), 4);

        launcher.showPage(ProjectLauncher::NewProject);
        QCOMPARE(launcher.m_heading->text(), QStringLiteral("Choose a template"));
        QVERIFY(!launcher.m_details->isHidden());
        QCOMPARE(launcher.m_templates->count(), 2);
        launcher.m_templates->setCurrentRow(1);
        launcher.m_location->setText(location);
        launcher.m_name->setText(QString());
        QVERIFY(!launcher.m_choose->isEnabled());
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("Name the project"));
        launcher.m_name->setText(QStringLiteral("Taken"));
        QVERIFY(!launcher.m_choose->isEnabled());
        QVERIFY(launcher.m_problem->text().contains(QStringLiteral("already in that location")));
        launcher.m_name->setText(QStringLiteral("My Film"));
        QVERIFY(launcher.m_choose->isEnabled());
        QVERIFY(launcher.m_destination->text().endsWith(QStringLiteral("Projects/My Film")));
        launcher.m_remote->setText(QStringLiteral("not an address"));
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("The remote must be a Git address"));
        QVERIFY(launcher.m_git->isChecked());
        launcher.m_remote->clear();
        launcher.m_rate->setCurrentIndex(launcher.m_rate->findData(QStringLiteral("24")));

        // A private GitHub repository needs a signed-in GitHub CLI.
        QVERIFY(!launcher.m_github->isEnabled());
        launcher.setTools(QJsonObject{{"git", true}, {"gitLfs", true}, {"gh", true}, {"ghUser", "jun"}});
        QVERIFY(launcher.m_github->isEnabled());
        QVERIFY(launcher.m_gitHint->text().contains(QStringLiteral("@jun")));
        launcher.m_github->setChecked(true);
        QVERIFY(launcher.m_git->isChecked());
        QVERIFY(!launcher.m_remote->isEnabled());
        launcher.m_choose->click();
        QCOMPARE(launcher.m_requests.last(), (QJsonObject{{"type", "create"}, {"name", "My Film"}, {"location", location},
                                                          {"template", "vertical"}, {"rate", "24"}, {"git", true},
                                                          {"github", QJsonObject{{"private", true}}},
                                                          // Windows puts the drive in front.
                                                          {"kerfd", QFileInfo(QStringLiteral("/opt/kerf/bin/kerfd")).absoluteFilePath()}}));
        QVERIFY(launcher.m_busy);
        QVERIFY(!launcher.m_choose->isEnabled());
        QVERIFY(!launcher.m_progress->isHidden());
        // The bridge answers a failed task; the launcher says why and lets one try again.
        launcher.handleTaskStatus(QJsonObject{{"status", "failed"}, {"error", "not implemented yet"}});
        QVERIFY(!launcher.m_busy);
        QVERIFY(launcher.m_choose->isEnabled());
        QCOMPARE(launcher.m_problem->text(), QStringLiteral("not implemented yet"));
        QCOMPARE(launcher.m_problem->property("state").toString(), QStringLiteral("error"));
        launcher.handleTaskStatus(QJsonObject{{"status", "done"}, {"path", QDir(location).filePath("My Film")},
                                              {"warnings", QJsonArray{"Git LFS is not installed"}}});
        QCOMPARE(launcher.result(), int(QDialog::Accepted));
        QCOMPARE(launcher.chosenPath(), QDir(location).filePath(QStringLiteral("My Film")));

        // Recent projects, with what kerf_project_inspect says of them.
        ProjectLauncher recent;
        recent.m_inspectProject = [old](const QJsonObject &request) {
            const QString path = request.value("path").toString();
            if (path == old) return QJsonObject{{"path", path}, {"exists", false}};
            return QJsonObject{{"path", path}, {"exists", true}, {"project", true}, {"name", "Taken Film"},
                               {"branch", "main"}, {"presets", 2}};
        };
        recent.reloadRecent();
        recent.showPage(ProjectLauncher::Recent);
        QVERIFY(recent.m_details->isHidden());
        QCOMPARE(recent.m_recent->count(), 2);
        QVERIFY(recent.m_recent->item(1)->text().startsWith(QStringLiteral("Taken Film\n")));
        QVERIFY(recent.m_recent->item(1)->text().contains(QStringLiteral("⎇ main")));
        QVERIFY(recent.m_recent->item(1)->text().contains(QStringLiteral("2 presets")));
        recent.m_recent->setCurrentRow(0);
        QVERIFY(!recent.m_choose->isEnabled());
        QCOMPARE(recent.m_problem->text(), QStringLiteral("That project folder is no longer there"));
        recent.m_recent->setCurrentRow(1);
        recent.m_choose->click();
        QCOMPARE(recent.result(), int(QDialog::Accepted));
        QCOMPARE(recent.chosenPath(), taken);
        QCOMPARE(recent.m_requests.size(), 1);

        // Clone from a Git address into a free folder.
        ProjectLauncher clone;
        clone.showPage(ProjectLauncher::Clone);
        clone.m_cloneLocation->setText(location);
        clone.m_cloneUrl->setText(QStringLiteral("film"));
        QVERIFY(!clone.m_choose->isEnabled());
        QCOMPARE(clone.m_problem->text(), QStringLiteral("Enter a Git address such as https://github.com/you/film.git"));
        clone.m_cloneUrl->setText(QStringLiteral("git@github.com:jun/Taken.git"));
        QVERIFY(clone.m_problem->text().contains(QStringLiteral("already in that location")));
        clone.m_cloneUrl->setText(QStringLiteral("https://github.com/jun/film.git"));
        QVERIFY(clone.m_choose->isEnabled());
        QVERIFY(clone.m_cloneDestination->text().endsWith(QStringLiteral("Projects/film")));
        clone.m_choose->click();
        QCOMPARE(clone.m_requests.last(), (QJsonObject{{"type", "clone"}, {"url", "https://github.com/jun/film.git"}, {"location", location}}));

        // From a preset: each slot needs a file that fits it.
        ProjectLauncher fromPreset;
        fromPreset.m_inspectPreset = [](const QJsonObject &request) {
            if (!request.contains("files"))
                return QJsonObject{{"name", "Intro"}, {"texts", QJsonObject{{"c4", "Title"}}}, {"slots", QJsonArray{
                    QJsonObject{{"id", "slot1"}, {"seconds", 4}, {"video", true}, {"audio", true}, {"clips", 2}},
                    QJsonObject{{"id", "slot2"}, {"seconds", 1}, {"video", true}, {"audio", false}, {"clips", 1}}}}};
            QJsonArray files;
            for (const QJsonValue &file : request.value("files").toArray()) {
                const bool longer = file.toString().endsWith(QStringLiteral("long.mov"));
                files.append(QJsonObject{{"path", file}, {"seconds", longer ? 10 : 2}, {"video", true}, {"audio", true},
                                         {"fits", longer ? QJsonArray{"slot1", "slot2"} : QJsonArray{"slot2"}}});
            }
            return QJsonObject{{"files", files}};
        };
        fromPreset.showPage(ProjectLauncher::FromPreset);
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("Choose a preset"));
        fromPreset.m_name->setText(QStringLiteral("Teaser"));
        fromPreset.m_location->setText(location);
        fromPreset.loadPreset(QStringLiteral("/library/intro.kerfpreset"));
        QVERIFY(fromPreset.findChild<QPushButton *>(QStringLiteral("presetSlotFile_slot1")));
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("Choose a file for each media slot"));
        fromPreset.setSlotFile(QStringLiteral("slot1"), QStringLiteral("/clips/short.mov"));
        fromPreset.setSlotFile(QStringLiteral("slot2"), QStringLiteral("/clips/short.mov"));
        QCOMPARE(fromPreset.m_problem->text(), QStringLiteral("short.mov cannot fill slot1"));
        auto *status = fromPreset.findChild<QLabel *>(QStringLiteral("presetSlotStatus_slot1"));
        QVERIFY(status->text().contains(QStringLiteral("is shorter than 4.00 s")));
        QCOMPARE(status->property("fit").toString(), QStringLiteral("no"));
        auto *start = fromPreset.findChild<QDoubleSpinBox *>(QStringLiteral("presetSlotFrom_slot1"));
        QVERIFY(!start->isEnabled());
        fromPreset.setSlotFile(QStringLiteral("slot1"), QStringLiteral("/clips/long.mov"));
        QVERIFY(status->text().startsWith(QStringLiteral("✓ long.mov")));
        QVERIFY(fromPreset.m_choose->isEnabled());
        // The slot can start later in a longer file.
        QVERIFY(start->isEnabled());
        QCOMPARE(start->maximum(), 6.0);
        start->setValue(1.5);
        auto *title = fromPreset.findChild<QLineEdit *>(QStringLiteral("presetText_c4"));
        title->setText(QStringLiteral("New title"));
        title->setModified(true);
        fromPreset.m_choose->click();
        const QJsonObject request = fromPreset.m_requests.last();
        QCOMPARE(request.value("type").toString(), QStringLiteral("create"));
        QCOMPARE(request.value("name").toString(), QStringLiteral("Teaser"));
        QCOMPARE(request.value("preset").toObject(), (QJsonObject{
            {"path", "/library/intro.kerfpreset"},
            {"bindings", QJsonObject{{"slot1", QJsonObject{{"file", "/clips/long.mov"}, {"from", "1.50s"}}},
                                     {"slot2", QJsonObject{{"file", "/clips/short.mov"}}}}},
            {"texts", QJsonObject{{"c4", "New title"}}}}));

        // File ▸ Open Recent opens a project in a new window.
        MainWindow window{taken, QString()};
        window.buildRecentMenu(window.m_recentMenu);
        QAction *openOld = nullptr;
        for (QAction *action : window.m_recentMenu->actions())
            if (action->text() == QStringLiteral("Old Cut")) openOld = action;
        QVERIFY(openOld);
        QSignalSpy opened(&window, &MainWindow::projectOpenRequested);
        openOld->trigger();
        QCOMPARE(opened.count(), 1);
        QCOMPARE(opened.first().first().toString(), old);

        ProjectLauncher::clearRecentProjects();
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
    }

    // The launcher draws the editor's graphite ramp with its own style sheet,
    // dark only, on every page: the sidebar on chrome, the pages on card, the
    // default button the flat accent. With KERF_SCREENSHOT_DIR set, each page
    // is saved as <page>-dark.png.
    void launcherRendersTheGraphiteRamp() {
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
        QTemporaryDir root;
        QVERIFY(root.isValid());
        QStringList projects;
        for (const char *name : {"Harbor Teaser", "Night Market", "Studio Session"}) {
            projects << QDir(root.path()).filePath(QString::fromLatin1(name));
            QVERIFY(QDir().mkpath(QDir(projects.last()).filePath(QStringLiteral("presets"))));
        }
        QFile presetFile(QDir(projects.first()).filePath(QStringLiteral("presets/Intro Titles.kerfpreset")));
        QVERIFY(presetFile.open(QIODevice::WriteOnly));
        presetFile.write("kerf 1\n");
        presetFile.close();
        ProjectLauncher::clearRecentProjects();
        for (auto it = projects.crbegin(); it != projects.crend(); ++it) ProjectLauncher::rememberProject(*it);
        const auto inspectProject = [projects](const QJsonObject &request) {
            const QString path = request.value("path").toString();
            const int index = std::max(0, int(projects.indexOf(path)));
            static const char *const branches[] = {"main", "color-pass", "main"};
            static const char *const edited[] = {"2026-10-30T18:42:00Z", "2026-10-27T09:15:00Z", "2026-10-21T21:03:00Z"};
            return QJsonObject{{"path", path}, {"exists", true}, {"project", true}, {"name", QFileInfo(path).fileName()},
                               {"branch", branches[index]}, {"modified", edited[index]}, {"presets", index == 0 ? 1 : 0}};
        };
        const auto inspectPreset = [](const QJsonObject &request) {
            if (!request.contains("files"))
                return QJsonObject{{"name", "Intro Titles"}, {"texts", QJsonObject{{"c4", "Harbor Teaser"}}}, {"slots", QJsonArray{
                    QJsonObject{{"id", "Opening"}, {"seconds", 2}, {"video", true}, {"audio", true}, {"clips", 2}},
                    QJsonObject{{"id", "Logo"}, {"seconds", 1.5}, {"video", true}, {"audio", false}, {"clips", 1}}}}};
            QJsonArray files;
            for (const QJsonValue &file : request.value("files").toArray())
                files.append(QJsonObject{{"path", file}, {"seconds", 6}, {"video", true}, {"audio", true},
                                         {"fits", QJsonArray{"Opening", "Logo"}}});
            return QJsonObject{{"files", files}};
        };
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        // Nothing amber or gold is left from the editor's palette.
        QPoint firstAmber;
        {
            QImage probe(40, 40, QImage::Format_RGB32);
            probe.fill(Qt::white);
            QPainter painter(&probe);
            painter.fillRect(QRect(0, 0, 1, 40), QColor(0xf0, 0xb4, 0x29));
            QCOMPARE(amberPixels(probe), 0);
            painter.fillRect(QRect(10, 10, 20, 20), QColor(0xf0, 0xb4, 0x29));
            QVERIFY(amberPixels(probe) > 0);
        }
        const char *const pageNames[] = {"new", "recent", "preset", "clone"};
        {
            const QString schemeName = QStringLiteral("dark");
            for (int page = ProjectLauncher::NewProject; page <= ProjectLauncher::Clone; ++page) {
                QSettings().remove(QStringLiteral("launcher"));
                ProjectLauncher launcher;
                // Dark only: the launcher draws the one theme.
                QCOMPARE(launcher.theme().accent, launcherTheme().accent);
                launcher.m_inspectProject = inspectProject;
                launcher.m_inspectPreset = inspectPreset;
                launcher.reloadRecent();
                launcher.reloadPresetChoices();
                launcher.setTools(QJsonObject{{"git", true}, {"gitLfs", true}, {"gh", true}, {"ghUser", "jun"}});
                QCOMPARE(launcher.m_recent->count(), 3);
                launcher.showPage(static_cast<ProjectLauncher::Page>(page));
                if (page == ProjectLauncher::FromPreset) {
                    launcher.m_presetChoices->setCurrentRow(0);
                    QCOMPARE(launcher.m_slotOrder, (QStringList{"Opening", "Logo"}));
                    launcher.setSlotFile(QStringLiteral("Opening"), QStringLiteral("/Volumes/Shoot/harbor-wide.mov"));
                } else if (page == ProjectLauncher::Clone) {
                    launcher.m_cloneUrl->setText(QStringLiteral("https://github.com/jun/harbor-teaser.git"));
                }
                launcher.resize(1040, 650);
                launcher.show();
                QVERIFY(QTest::qWaitForWindowExposed(&launcher));
                // A chosen recent project is the selection while its list has focus.
                if (page == ProjectLauncher::Recent) launcher.m_recent->setFocus();
                QCoreApplication::processEvents();
                QImage image = launcher.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const auto at = [&image, ratio](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio).rgb(); };
                QCOMPARE(at(launcher.width() - 6, 6), launcher.theme().content.rgb());
                QCOMPARE(at(100, launcher.height() - 10), launcher.theme().sidebar.rgb());
                // The graphite ramp, exactly: card pages, a chrome sidebar.
                QCOMPARE(launcher.theme().content.rgb(), QColor("#1c1c1c").rgb());
                QCOMPARE(launcher.theme().sidebar.rgb(), QColor("#171717").rgb());
                QCOMPARE(launcher.theme().field.rgb(), QColor("#262626").rgb());
                QVERIFY(isGraphite(launcher.theme().content));
                QVERIFY(isGraphite(launcher.theme().sidebar));
                // Controls are the design's 24pt.
                QCOMPARE(launcher.m_choose->height(), 24);
                if (launcher.m_choose->isEnabled()) {
                    // The default button is the flat accent, top to foot.
                    const QImage choose = launcher.m_choose->grab().toImage();
                    const qreal r = choose.devicePixelRatio();
                    const QColor accent = launcher.theme().accent;
                    for (const int y : {4, launcher.m_choose->height() / 2, launcher.m_choose->height() - 4})
                        QCOMPARE(choose.pixelColor(QPoint(8, y) * r).rgb(), accent.rgb());
                }
                QVERIFY2(amberPixels(image, &firstAmber) == 0,
                         qPrintable(QStringLiteral("amber area in %1-%2 at %3,%4")
                                        .arg(QString::fromLatin1(pageNames[page]), schemeName)
                                        .arg(firstAmber.x())
                                        .arg(firstAmber.y())));
#if defined(Q_OS_MACOS) && QT_VERSION >= QT_VERSION_CHECK(6, 9, 0)
                // The sidebar runs up under a transparent title bar; the content titles itself.
                QVERIFY(launcher.windowFlags().testFlag(Qt::ExpandedClientAreaHint));
                QCOMPARE(launcher.m_title->text(), launcher.windowTitle());
                QCOMPARE(at(launcher.width() - 6, launcher.m_title->height() - 1), launcher.theme().titleRule.rgb());
#endif
                save(image, QStringLiteral("%1-%2").arg(QString::fromLatin1(pageNames[page]), schemeName));
                if (page == ProjectLauncher::NewProject) {
                    // Details are folded away until opened, as in macOS.
                    QVERIFY(launcher.m_detailsBody->isHidden());
                    // Its chevron, the only sign of whether they are open,
                    // reads on the content (3:1), closed and open.
                    QCOMPARE(ink(launcher.m_detailsToggle->icon()), launcher.theme().secondary.rgb());
                    launcher.m_detailsToggle->click();
                    QVERIFY(!launcher.m_detailsBody->isHidden());
                    QCOMPARE(ink(launcher.m_detailsToggle->icon()), launcher.theme().secondary.rgb());
                    launcher.m_git->setChecked(true);
                    launcher.m_name->setFocus();
                    QCoreApplication::processEvents();
                    image = launcher.grab().toImage();
                    QVERIFY2(amberPixels(image, &firstAmber) == 0,
                             qPrintable(QStringLiteral("amber area in new-details-%1 at %2,%3")
                                            .arg(schemeName)
                                            .arg(firstAmber.x())
                                            .arg(firstAmber.y())));
                    save(image, QStringLiteral("new-details-%1").arg(schemeName));
                    // The remote field's placeholder reads at 4.5:1 in the
                    // launcher's secondary text colour, and dims when the field is off.
                    const LauncherTheme &look = launcher.theme();
                    launcher.m_remote->ensurePolished();
                    QVERIFY(launcher.m_remote->isEnabled() && launcher.m_remote->text().isEmpty());
                    for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive})
                        QCOMPARE(launcher.m_remote->palette().color(group, QPalette::PlaceholderText).rgb(),
                                 look.secondary.rgb());
                    QVERIFY(contrastRatio(launcher.m_remote->palette().color(QPalette::Active, QPalette::PlaceholderText),
                                          look.field) >= 4.5);
                    QCOMPARE(launcher.m_remote->palette().color(QPalette::Disabled, QPalette::PlaceholderText).rgb(),
                             look.disabledText.rgb());
                    // Its frame-rate pop-up opens with every choice showing.
                    QVERIFY(popupShowsEveryRow(launcher.m_rate));
                }
                launcher.close();
            }
        }
        ProjectLauncher::clearRecentProjects();
        QSettings().remove(QStringLiteral("launcher"));
        QSettings().remove(QStringLiteral("projects"));
    }

    // A chosen recent project's text reads on its selection (4.5:1): a
    // missing project's name on the on fill of a list without focus, and the
    // details on the focused list's accent selection. The colours are read
    // from the pens the rows are painted with.
    void launcherRowsReadWhenChosen() {
        const auto clean = qScopeGuard([] {
            ProjectLauncher::clearRecentProjects();
            QSettings().remove(QStringLiteral("launcher"));
            QSettings().remove(QStringLiteral("projects"));
        });
        QTemporaryDir root;
        QVERIFY(root.isValid());
        const QString present = QDir(root.path()).filePath(QStringLiteral("Harbor Teaser"));
        QVERIFY(QDir().mkpath(present));
        const QString missing = QDir(root.path()).filePath(QStringLiteral("Gone Project"));
        ProjectLauncher::clearRecentProjects();
        ProjectLauncher::rememberProject(present);
        ProjectLauncher::rememberProject(missing);
        {
            QSettings().remove(QStringLiteral("launcher"));
            ProjectLauncher launcher;
            launcher.m_inspectProject = [](const QJsonObject &request) {
                const QString path = request.value("path").toString();
                return QJsonObject{{"path", path}, {"exists", QFileInfo(path).isDir()}, {"name", QFileInfo(path).fileName()},
                                   {"branch", "main"}};
            };
            launcher.reloadRecent();
            const LauncherTheme &t = launcher.theme();
            QListWidget *list = launcher.m_recent;
            QCOMPARE(list->count(), 2);
            // The colour `text` is painted in on row `row`, chosen.
            const auto painted = [list](int row, const QString &text) {
                QStyleOptionViewItem option;
                option.initFrom(list);
                option.widget = list;
                option.font = list->font();
                option.rect = QRect(0, 0, 600, 52);
                option.state |= QStyle::State_Selected;
                TextRecorder recorder;
                QPainter painter(&recorder);
                list->itemDelegate()->paint(&painter, option, list->model()->index(row, 0));
                painter.end();
                return recorder.colorOf(text);
            };
            // Missing, chosen, in a list without focus (as when the sidebar has it).
            QVERIFY(!list->hasFocus());
            QCOMPARE(list->item(0)->text().section(QLatin1Char('\n'), 0, 0), QStringLiteral("Gone Project"));
            const QColor name = painted(0, QStringLiteral("Gone Project"));
            QVERIFY(name.isValid());
            QVERIFY2(contrastRatio(laidOn(t.selectionInactive, name), t.selectionInactive) >= 4.5, qPrintable(name.name()));
            // Present, chosen, in the focused list.
            launcher.showPage(ProjectLauncher::Recent);
            launcher.show();
            QVERIFY(QTest::qWaitForWindowExposed(&launcher));
            launcher.activateWindow();
            list->setFocus();
            QTRY_VERIFY(list->hasFocus());
            const QColor branch = painted(1, QStringLiteral("main"));
            QVERIFY(branch.isValid());
            QVERIFY2(contrastRatio(laidOn(t.selection, branch), t.selection) >= 4.5, qPrintable(branch.name(QColor::HexArgb)));
            launcher.close();
        }
    }

    // The start window: the folders, one question, and a request on its way.
    void theStartWindowSendsARequestToAFolder() {
        ProjectLauncher::clearRecentProjects();
        QTemporaryDir folder;
        QVERIFY(folder.isValid());
        HomeWindow home;
        QVERIFY(home.chosenPath().isEmpty());
        QVERIFY(!home.m_composer->sendButton()->isEnabled());
        QVERIFY(home.m_openOnly->isHidden());
        QVERIFY(home.m_composer->stamp()->isHidden());
        // The suggestions fill the composer; there is no folder yet, so sending asks for one.
        home.m_ideas.first()->click();
        QCOMPARE(home.m_composer->text(), HomeWindow::suggestions().first());
        QVERIFY(home.m_composer->sendButton()->isEnabled());
        int asked = 0;
        home.m_pickFolder = [&asked] {
            ++asked;
            return QString();
        };
        home.send();
        QCOMPARE(asked, 1);
        QCOMPARE(home.result(), int(QDialog::Rejected));
        home.m_pickFolder = [&folder] { return folder.path(); };
        home.send();
        QCOMPARE(home.result(), int(QDialog::Accepted));
        QCOMPARE(home.chosenPath(), QDir::cleanPath(QFileInfo(folder.path()).absoluteFilePath()));
        QCOMPARE(home.request(), HomeWindow::suggestions().first());
        QVERIFY(home.startsProject());
    }

    void theStartWindowListsRecentFoldersAndOpensOne() {
        ProjectLauncher::clearRecentProjects();
        QTemporaryDir older;
        QTemporaryDir newer;
        QVERIFY(older.isValid() && newer.isValid());
        QFile project(QDir(older.path()).filePath(QStringLiteral("project.kerf")));
        QVERIFY(project.open(QIODevice::WriteOnly));
        project.close();
        ProjectLauncher::rememberProject(older.path());
        ProjectLauncher::rememberProject(newer.path());
        {
            HomeWindow home;
            // The most recent is chosen; a row chooses another.
            QCOMPARE(home.chosenPath(), QDir::cleanPath(QFileInfo(newer.path()).absoluteFilePath()));
            QCOMPARE(home.m_sidebar->projects().size(), 2);
            QVERIFY(!home.m_openOnly->isHidden());
            home.m_sidebar->projectRow(older.path())->click();
            QCOMPARE(home.chosenPath(), QDir::cleanPath(QFileInfo(older.path()).absoluteFilePath()));
            QVERIFY(home.m_folderChip->text() == QFileInfo(older.path()).fileName());
            // Opening without asking: no request, and it is a project already.
            home.m_openOnly->click();
            QCOMPARE(home.result(), int(QDialog::Accepted));
            QVERIFY(home.request().isEmpty());
            QVERIFY(!home.startsProject());
        }
        ProjectLauncher::clearRecentProjects();
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::LauncherTest)

#include "launcher_test.moc"
