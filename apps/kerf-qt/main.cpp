#include "editor_style.h"
#include "main_window.h"

#include <QApplication>
#include <QDir>
#include <QFontDatabase>
#include <QTimer>

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
    if (project.isEmpty()) project = QDir::currentPath();
    kerfd = positional.value(1, qEnvironmentVariable("KERFD_BIN"));
    if (kerfd.isEmpty()) kerfd = qEnvironmentVariable("KERFD_BINARY");
    if (kerfd.isEmpty()) kerfd = QStringLiteral("kerfd");
    kerf::MainWindow window(project, kerfd);
    window.show();
    if (renderPreview) {
        QTimer::singleShot(1500, &window, [&window] {
            QMetaObject::invokeMethod(&window, "renderPreview", Qt::DirectConnection);
        });
    }
    if (!screenshotPath.isEmpty() || quitAfterMs > 0) {
        const int delay = quitAfterMs > 0 ? quitAfterMs : 500;
        QTimer::singleShot(delay, &app, [&window, &app, screenshotPath] {
            if (!screenshotPath.isEmpty()) window.grab().save(screenshotPath);
            app.quit();
        });
    }
    return app.exec();
}
