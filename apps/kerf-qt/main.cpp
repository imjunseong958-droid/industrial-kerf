#include "app_icon.h"
#include "bridge.h"
#include "crash_notice.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "link_inbox.h"
#include "main_window.h"
#include "project_launcher.h"
#include "project_link.h"
#include "secret_store.h"
#include "settings_migration.h"
#include "update_notice.h"

#include <QApplication>
#include <QDir>
#include <QJsonObject>
#include <QPointer>
#include <QRect>
#include <QSettings>
#include <QSize>
#include <QStandardPaths>
#include <QTimer>

namespace {

// An editor window for `project`. It lives on the heap and is deleted when
// closed; opening another project from it opens a new window first, so the
// application keeps running.
kerf::MainWindow *openEditor(const QString &project, const QString &kerfd, const QRect &geometry = QRect()) {
    // `project` may be empty: an editor with no folder yet, which offers them.
    kerf::ProjectLauncher::rememberProject(project);
    auto *window = new kerf::MainWindow(project, kerfd);
    window->setAttribute(Qt::WA_DeleteOnClose);
    // In the place of the window it follows.
    if (geometry.isValid()) window->setGeometry(geometry);
    QObject::connect(window, &kerf::MainWindow::projectOpenRequested, window, [window, kerfd](const QString &path, bool starts) {
        kerf::MainWindow *next = openEditor(path, kerfd, window->geometry());
        // A folder that was no project yet: its media comes in with it.
        if (starts) next->startWith(QString(), true);
        window->close();
    });
    // The same, on one of that project's conversations.
    QObject::connect(window, &kerf::MainWindow::conversationOpenRequested, window,
                     [window, kerfd](const QString &path, const QString &conversation) {
                         openEditor(path, kerfd, window->geometry())->openConversation(conversation);
                         window->close();
                     });
    window->show();
    return window;
}

} // namespace

int main(int argc, char **argv) {
    // Run as Kerf's tools for Claude Code (--serve-mcp): no application, no window.
    if (const int served = kerf_serve_mcp(argc - 1, argv + 1); served >= 0) return served;
#ifdef Q_OS_MACOS
    // Before any thread starts, and for every tool the editor and kerfd run.
    qputenv("PATH", kerf::pathWithToolFolders(qEnvironmentVariable("PATH"), QDir::homePath()).toLocal8Bit());
#endif
    // A smoke run may keep its settings in a folder of its own (INI files),
    // so a capture neither reads nor changes the user's: set before the
    // first QSettings is made. Its keys stay in that folder too, as where no
    // keychain can be reached, so it never asks for the user's.
    for (int i = 1; i + 1 < argc; ++i)
        if (QString::fromUtf8(argv[i]) == QStringLiteral("--settings-dir")) {
            QSettings::setDefaultFormat(QSettings::IniFormat);
            QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, QString::fromUtf8(argv[i + 1]));
            kerf::secrets::call() = [](const QJsonObject &) { return QJsonObject{}; };
        }
    // A panic in the editor's own Rust leaves a report, as kerfd's and the CLI's
    // do, and so does a crash in its C++ or Qt, and a fatal message from Qt
    // such as a platform plugin that would not load.
    kerf_crash_install();
    kerf::reportFatalMessages();
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Industrial Kerf"));
    // Windows and Linux add it after the project's name in window titles;
    // macOS titles a window with its document alone.
    QApplication::setApplicationDisplayName(QStringLiteral("Industrial Kerf"));
    QApplication::setOrganizationName(QStringLiteral("Industrial Kerf"));
#ifndef Q_OS_MACOS
    // Kerf's icon on every window and in the taskbar. On macOS the Dock shows
    // the app's own (CFBundleIconFile), drawn to the Dock's grid, and setting
    // one here would put this tile, drawn without that margin, in its place.
    QApplication::setWindowIcon(kerf::appIcon());
#endif
    // Before anything reads them: settings from an older Kerf come up to
    // this one's version.
    {
        QSettings settings;
        kerf::migrateSettings(settings);
    }
    // The design's faces: Inter for the interface and JetBrains Mono for
    // figures, compiled into the app.
    kerf::loadBundledFonts();
    // And the ones chosen in Settings, before anything is set in them.
    kerf::tokens::faces() = kerf::facesFor(kerf::loadFaceChoice());
    app.setFont(kerf::tokens::font(kerf::tokens::body));
    // Fusion with the design's flat metrics and focus outline.
    kerf::useEditorStyle();
    // The editor is dark only; this applies its one theme.
    kerf::applyEditorAppearance();

    QString project;
    QString kerfd;
    QString screenshotPath;
    bool renderPreview = false;
    int quitAfterMs = 0;
    QSize windowSize;
    QStringList positional;
    QStringList links;
    for (int i = 1; i < argc; ++i) {
        const QString argument = QString::fromUtf8(argv[i]);
        if (argument == QStringLiteral("--render-preview")) {
            renderPreview = true;
        } else if (argument == QStringLiteral("--screenshot") && i + 1 < argc) {
            screenshotPath = QString::fromUtf8(argv[++i]);
        } else if (argument.startsWith(QStringLiteral("--screenshot="))) {
            screenshotPath = argument.mid(13);
        } else if (argument == QStringLiteral("--quit-after-ms") && i + 1 < argc) {
            quitAfterMs = QString::fromUtf8(argv[++i]).toInt();
        } else if (argument.startsWith(QStringLiteral("--quit-after-ms="))) {
            quitAfterMs = argument.mid(16).toInt();
        } else if (argument == QStringLiteral("--settings-dir") && i + 1 < argc) {
            ++i; // applied before the application was made
        } else if (argument == QStringLiteral("--window-size") && i + 1 < argc) {
            // WxH, e.g. 1600x1000 (the design's canvas).
            const QStringList parts = QString::fromUtf8(argv[++i]).split(QLatin1Char('x'));
            if (parts.size() == 2) windowSize = QSize(parts.at(0).toInt(), parts.at(1).toInt());
        } else if (kerf::links::isProjectLink(argument)) {
            // kerf://<project>/rev<N>[@TC], from a shell (macOS hands a
            // clicked link over as a QFileOpenEvent instead).
            links << argument;
        } else {
            positional << argument;
        }
    }
    project = positional.value(0, qEnvironmentVariable("KERF_PROJECT_DIR"));
    if (project.isEmpty()) project = qEnvironmentVariable("KERF_PROJECT_DIRECTORY");
    kerfd = positional.value(1, qEnvironmentVariable("KERFD_BIN"));
    if (kerfd.isEmpty()) kerfd = qEnvironmentVariable("KERFD_BINARY");
    // Otherwise the kerfd beside the editor (the installed bundle carries
    // one, and Finder and the Dock give an app no shell PATH), else the one
    // on the PATH.
    if (kerfd.isEmpty()) kerfd = QStandardPaths::findExecutable(QStringLiteral("kerfd"), {QCoreApplication::applicationDirPath()});
    if (kerfd.isEmpty()) kerfd = QStringLiteral("kerfd");
    const bool smoke = !screenshotPath.isEmpty() || quitAfterMs > 0;
#ifndef Q_OS_MACOS
    // Windows and Linux start an editor for each link a person follows: an
    // editor already open takes the links, as macOS's does, and this one,
    // started for them alone, ends before it shows anything. A run that
    // captures, quits on its own or previews neither hands links over nor
    // takes them.
    const bool takesLinks = !smoke && !renderPreview;
    QString inboxName;
    if (takesLinks) {
        QSettings settings;
        inboxName = kerf::links::inboxName(settings);
    }
    if (takesLinks && !links.isEmpty() && positional.isEmpty() && kerf::links::handOff(inboxName, links)) return 0;
#endif
    // Since it was last open, did anything of Kerf crash? Not in a run that
    // captures or quits on its own, which nobody is there to answer.
    if (!smoke && !renderPreview) {
        QSettings settings;
        kerf::showCrashNotice(nullptr, settings);
    }
    // A newer Kerf on GitHub, asked off the GUI thread at most once a day.
    if (!smoke && !renderPreview) kerf::startUpdateCheck(nullptr);

    // kerf:// project links raise (or open) their project's editor and seek.
    kerf::links::Router router(
        [&kerfd](const QString &directory) -> kerf::MainWindow * { return openEditor(directory, kerfd); },
        [] {
            QList<kerf::MainWindow *> windows;
            for (QWidget *widget : QApplication::topLevelWidgets())
                if (auto *window = qobject_cast<kerf::MainWindow *>(widget); window && !window->isHidden()) windows << window;
            return windows;
        });
    app.installEventFilter(&router);
#ifndef Q_OS_MACOS
    // Links the editors started for them hand over, while this one is open.
    kerf::links::Inbox inbox;
    if (takesLinks) {
        QObject::connect(&inbox, &kerf::links::Inbox::linkReceived, &router,
                         [&router](const QString &link) { router.follow(link); });
        inbox.listen(inboxName);
    }
#endif
    // Without a project, Kerf opens on what was being worked on, as a coding
    // agent's app does: the latest project that is still there. With none to
    // go back to, the editor opens on a new conversation, which offers the
    // folders, Open Folder and New Project.
    QPointer<QWidget> shown;
    if (project.isEmpty() && !links.isEmpty()) {
        // Opened for a link: its project is the one to show.
        for (const QString &link : std::as_const(links))
            if (kerf::MainWindow *window = router.follow(link)) shown = window;
        links.clear();
    }
    // A run that captures or quits on its own opens no project it was not
    // given: its picture is the same on every machine, and nothing of the
    // user's is opened.
    if (!shown && project.isEmpty() && !smoke) project = kerf::ProjectLauncher::lastProject();
    if (!shown) {
        QPointer<kerf::MainWindow> window = openEditor(project.isEmpty() ? QString() : QDir(project).absolutePath(), kerfd);
        if (windowSize.isValid() && !windowSize.isEmpty()) window->resize(windowSize);
        shown = window.data();
        if (renderPreview && !project.isEmpty()) {
            QTimer::singleShot(1500, window.data(), [window] {
                QMetaObject::invokeMethod(window.data(), "renderPreview", Qt::DirectConnection);
            });
        }
    }
    // Links given beside a project: followed once its editor exists.
    for (const QString &link : std::as_const(links)) router.follow(link);
    if (smoke) {
        const int delay = quitAfterMs > 0 ? quitAfterMs : 500;
        QTimer::singleShot(delay, &app, [shown, &app, screenshotPath] {
            if (shown && !screenshotPath.isEmpty()) shown->grab().save(screenshotPath);
            app.quit();
        });
    }
    return app.exec();
}
