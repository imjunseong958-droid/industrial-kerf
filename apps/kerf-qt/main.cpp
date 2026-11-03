#include "design_tokens.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "main_window.h"
#include "project_launcher.h"

#include <QApplication>
#include <QDir>
#include <QDirIterator>
#include <QFontDatabase>
#include <QPointer>
#include <QSettings>
#include <QSize>
#include <QStandardPaths>
#include <QTimer>

namespace {

// Register every font file under :/fonts (resources/fonts, compiled in only
// when that folder holds .ttf or .otf files).
void loadBundledFonts() {
    QDirIterator files(QStringLiteral(":/fonts"), {QStringLiteral("*.ttf"), QStringLiteral("*.otf")}, QDir::Files,
                       QDirIterator::Subdirectories);
    while (files.hasNext()) QFontDatabase::addApplicationFont(files.next());
}

// An editor window for `project`. It lives on the heap and is deleted when
// closed; opening another project from it opens a new window first, so the
// application keeps running.
kerf::MainWindow *openEditor(const QString &project, const QString &kerfd) {
    kerf::ProjectLauncher::rememberProject(project);
    auto *window = new kerf::MainWindow(project, kerfd);
    window->setAttribute(Qt::WA_DeleteOnClose);
    QObject::connect(window, &kerf::MainWindow::projectOpenRequested, window, [window, kerfd](const QString &path) {
        openEditor(path, kerfd);
        window->close();
    });
    window->show();
    return window;
}

} // namespace

int main(int argc, char **argv) {
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
    // figures, from the bundle's :/fonts when it carries them. Without them
    // the interface is the system's UI font and figures Menlo, at the same
    // sizes, with tabular figures (design_tokens.h).
    loadBundledFonts();
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
            // A built-in or saved workspace's name: Review, Edit, Finish, Deliver.
            workspace = QString::fromUtf8(argv[++i]);
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

    // Without a project, choose one first. A smoke run shows the launcher
    // and captures it instead of waiting for a choice.
    QPointer<QWidget> shown;
    if (project.isEmpty() && smoke) {
        auto *launcher = new kerf::ProjectLauncher(kerfd);
        launcher->setAttribute(Qt::WA_DeleteOnClose);
        launcher->show();
        shown = launcher;
    } else if (project.isEmpty()) {
        kerf::ProjectLauncher launcher(kerfd);
        // Closing the launcher is not the last window closing.
        app.setQuitOnLastWindowClosed(false);
        if (launcher.exec() != QDialog::Accepted) return 0;
        app.setQuitOnLastWindowClosed(true);
        project = launcher.chosenPath();
    }
    if (!shown) {
        // The workspace to open in, as if chosen when the editor last closed.
        if (!workspace.isEmpty()) QSettings().setValue(QStringLiteral("workspace/current"), workspace);
        QPointer<kerf::MainWindow> window = openEditor(QDir(project).absolutePath(), kerfd);
        if (windowSize.isValid() && !windowSize.isEmpty()) window->resize(windowSize);
        shown = window.data();
        if (renderPreview) {
            QTimer::singleShot(1500, window.data(), [window] {
                QMetaObject::invokeMethod(window.data(), "renderPreview", Qt::DirectConnection);
            });
        }
    }
    if (smoke) {
        const int delay = quitAfterMs > 0 ? quitAfterMs : 500;
        QTimer::singleShot(delay, &app, [shown, &app, screenshotPath] {
            if (shown && !screenshotPath.isEmpty()) shown->grab().save(screenshotPath);
            app.quit();
        });
    }
    return app.exec();
}
