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
        QPointer<kerf::MainWindow> window = openEditor(QDir(project).absolutePath(), kerfd);
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
