// Help ▸ Kerf User Guide opens the page a package carries, and the guide on
// GitHub only where there is none.

#include "test_registry.h"

#include "user_guide.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QTemporaryDir>
#include <QTest>

namespace kerf {

namespace {

// A file at `path` under `root`, with something in it.
void place(const QString &root, const QString &path) {
    const QString file = QDir(root).filePath(path);
    QVERIFY(QDir().mkpath(QFileInfo(file).absolutePath()));
    QFile out(file);
    QVERIFY(out.open(QIODevice::WriteOnly));
    out.write("<!doctype html><title>Editing with Kerf</title>");
}

} // namespace

class UserGuideTest final : public QObject {
    Q_OBJECT
private slots:
    void theWindowsPackageHasItBesideTheEditor() {
        QTemporaryDir root;
        place(root.path(), QStringLiteral("Industrial-Kerf-0.1.1-windows-x64/user-guide.html"));
        const QString app = root.filePath(QStringLiteral("Industrial-Kerf-0.1.1-windows-x64"));
        const QString expected = root.filePath(QStringLiteral("Industrial-Kerf-0.1.1-windows-x64/user-guide.html"));
        QCOMPARE(localUserGuide(app), QDir::cleanPath(expected));
        QCOMPARE(userGuideUrl(app), QUrl::fromLocalFile(QDir::cleanPath(expected)));
    }

    void theMacAppHasItInItsResources() {
        QTemporaryDir root;
        place(root.path(), QStringLiteral("Industrial Kerf.app/Contents/Resources/user-guide.html"));
        const QString macos = root.filePath(QStringLiteral("Industrial Kerf.app/Contents/MacOS"));
        QVERIFY(QDir().mkpath(macos));
        const QString expected = root.filePath(QStringLiteral("Industrial Kerf.app/Contents/Resources/user-guide.html"));
        QCOMPARE(localUserGuide(macos), QDir::cleanPath(expected));
        QVERIFY(userGuideUrl(macos).isLocalFile());
    }

    void theLinuxPackageHasItInShareDoc() {
        QTemporaryDir root;
        place(root.path(), QStringLiteral("usr/share/doc/industrial-kerf/user-guide.html"));
        const QString bin = root.filePath(QStringLiteral("usr/bin"));
        QVERIFY(QDir().mkpath(bin));
        QCOMPARE(localUserGuide(bin), QDir::cleanPath(root.filePath(QStringLiteral("usr/share/doc/industrial-kerf/user-guide.html"))));
    }

    // A build from the repository carries none, and an empty file is not a guide.
    void elsewhereItIsTheGuideOnGitHub() {
        QTemporaryDir root;
        QCOMPARE(localUserGuide(root.path()), QString());
        QCOMPARE(userGuideUrl(root.path()), onlineUserGuideUrl());
        QFile empty(root.filePath(QStringLiteral("user-guide.html")));
        QVERIFY(empty.open(QIODevice::WriteOnly));
        empty.close();
        QCOMPARE(userGuideUrl(root.path()), onlineUserGuideUrl());
        QCOMPARE(userGuideUrl(QString()), onlineUserGuideUrl());
        QCOMPARE(onlineUserGuideUrl().host(), QStringLiteral("github.com"));
        QVERIFY(onlineUserGuideUrl().path().endsWith(QStringLiteral("/docs/user-guide.md")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::UserGuideTest)
#include "user_guide_test.moc"
