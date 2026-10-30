#include "editor_style.h"
#include "main_window.h"
#include "project_launcher.h"

#include <QApplication>
#include <QDir>
#include <QFontDatabase>
#include <QPointer>
#include <QTimer>

namespace {

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
    QApplication::setOrganizationName(QStringLiteral("Industrial Kerf"));
    QFont interfaceFont = QFontDatabase::systemFont(QFontDatabase::GeneralFont);
    if (QFontDatabase::families().contains(QStringLiteral("Avenir Next")))
        interfaceFont.setFamily(QStringLiteral("Avenir Next"));
    interfaceFont.setPixelSize(13);
    interfaceFont.setWeight(QFont::Normal);
    app.setFont(interfaceFont);
    app.setStyle(QStringLiteral("Fusion"));
    app.setStyleSheet(editorStyleSheet());

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
