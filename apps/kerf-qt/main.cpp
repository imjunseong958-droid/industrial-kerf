#include "design_tokens.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "home_window.h"
#include "main_window.h"
#include "project_launcher.h"
#include "review_link.h"

#include <QApplication>
#include <QDir>
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
    kerf::ProjectLauncher::rememberProject(project);
    auto *window = new kerf::MainWindow(project, kerfd);
    window->setAttribute(Qt::WA_DeleteOnClose);
    // In the place of the window it follows.
    if (geometry.isValid()) window->setGeometry(geometry);
    QObject::connect(window, &kerf::MainWindow::projectOpenRequested, window, [window, kerfd](const QString &path) {
        openEditor(path, kerfd);
        window->close();
    });
    window->show();
    return window;
}

} // namespace

int main(int argc, char **argv) {
#ifdef Q_OS_MACOS
    // Before any thread starts, and for every tool the editor and kerfd run.
    qputenv("PATH", kerf::pathWithToolFolders(qEnvironmentVariable("PATH"), QDir::homePath()).toLocal8Bit());
#endif
    // A smoke run may keep its settings in a folder of its own (INI files),
    // so a capture neither reads nor changes the user's: set before the
    // first QSettings is made.
    for (int i = 1; i + 1 < argc; ++i)
        if (QString::fromUtf8(argv[i]) == QStringLiteral("--settings-dir")) {
            QSettings::setDefaultFormat(QSettings::IniFormat);
            QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, QString::fromUtf8(argv[i + 1]));
        }
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Industrial Kerf"));
    // Windows and Linux add it after the project's name in window titles;
    // macOS titles a window with its document alone.
    QApplication::setApplicationDisplayName(QStringLiteral("Industrial Kerf"));
    QApplication::setOrganizationName(QStringLiteral("Industrial Kerf"));
    // The design's faces: Inter for the interface and JetBrains Mono for
    // figures, compiled into the app.
    kerf::loadBundledFonts();
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
    QString workspace;
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
        } else if (argument == QStringLiteral("--workspace") && i + 1 < argc) {
            // Review (the base state), or the tray to open over it: Finish, Deliver.
            workspace = QString::fromUtf8(argv[++i]);
        } else if (kerf::links::isReviewLink(argument)) {
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

    // kerf:// review links raise (or open) their project's editor and seek.
    QPointer<QDialog> choosing;
    kerf::links::Router router(
        [&kerfd, &choosing](const QString &directory) -> kerf::MainWindow * {
            // A link followed while the launcher waits stands for its choice.
            if (choosing) choosing->reject();
            return openEditor(directory, kerfd);
        },
        [] {
            QList<kerf::MainWindow *> windows;
            for (QWidget *widget : QApplication::topLevelWidgets())
                if (auto *window = qobject_cast<kerf::MainWindow *>(widget); window && !window->isHidden()) windows << window;
            return windows;
        });
    app.installEventFilter(&router);
    const auto anyEditor = [] {
        for (QWidget *widget : QApplication::topLevelWidgets())
            if (qobject_cast<kerf::MainWindow *>(widget) && !widget->isHidden()) return true;
        return false;
    };

    // Without a project, start from the folders and a question. A smoke run
    // shows that window and captures it instead of waiting for a choice.
    QPointer<QWidget> shown;
    QString startRequest;
    bool startImport = false;
    QRect startGeometry;
    if (project.isEmpty() && smoke) {
        auto *home = new kerf::HomeWindow(kerfd);
        home->setAttribute(Qt::WA_DeleteOnClose);
        if (windowSize.isValid() && !windowSize.isEmpty()) home->resize(windowSize);
        home->show();
        shown = home;
    } else if (project.isEmpty() && !links.isEmpty()) {
        // Opened for a link: its project is the one to show.
        for (const QString &link : std::as_const(links))
            if (kerf::MainWindow *window = router.follow(link)) shown = window;
        links.clear();
    }
    if (!shown && project.isEmpty()) {
        kerf::HomeWindow home(kerfd);
        choosing = &home;
        // Closing the window is not the last window closing.
        app.setQuitOnLastWindowClosed(false);
        const bool chosen = home.exec() == QDialog::Accepted;
        app.setQuitOnLastWindowClosed(true);
        if (!chosen && !anyEditor()) return 0;
        if (chosen) {
            project = home.chosenPath();
            startRequest = home.request();
            startImport = home.startsProject();
            startGeometry = home.geometry();
        }
    }
    if (!shown && !project.isEmpty()) {
        QPointer<kerf::MainWindow> window = openEditor(QDir(project).absolutePath(), kerfd, startGeometry);
        if (!workspace.isEmpty()) window->openTray(workspace);
        if (!startRequest.isEmpty() || startImport) window->startWith(startRequest, startImport);
        if (windowSize.isValid() && !windowSize.isEmpty()) window->resize(windowSize);
        shown = window.data();
        if (renderPreview) {
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
