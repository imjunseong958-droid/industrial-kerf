// Tests for kerf:// review links: reading them, finding their project, the
// router that raises or opens an editor (QFileOpenEvent and arguments), and
// the editor's seek once it is connected.

#include "fixtures.h"
#include "test_registry.h"

#include "main_window.h"
#include "project_launcher.h"
#include "review_link.h"
#include "wiring/review_wiring.h"
#include "ui/editor_bus.h"

#include <QApplication>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileOpenEvent>
#include <QSettings>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <QUrl>

#include <cmath>
#include <memory>
#include <vector>

namespace kerf {

using namespace test;

namespace {

// A project folder named `name` with a project.kerf in it.
QString makeProject(const QTemporaryDir &root, const QString &name) {
    const QString directory = QDir(root.path()).filePath(name);
    QDir().mkpath(directory);
    QFile file(QDir(directory).filePath(QStringLiteral("project.kerf")));
    if (file.open(QIODevice::WriteOnly)) file.write("kerf 1\n");
    return QDir::cleanPath(directory);
}

QJsonObject program(double duration, int revision) {
    QJsonObject snapshot = showcaseSnapshot(revision, QStringLiteral("clean"));
    snapshot.insert(QStringLiteral("duration"), duration);
    return snapshot;
}

} // namespace

class LinkTest final : public QObject {
    Q_OBJECT

private slots:
    void init() { ProjectLauncher::clearRecentProjects(); }
    void cleanup() { ProjectLauncher::clearRecentProjects(); }

    // kerf://<project>/rev<N>[@TC][?branch&head&remote], as the comments
    // panel's Copy link writes them.
    void linksReadAsTheEditorWritesThem() {
        using links::parse;
        QVERIFY(links::isReviewLink(QStringLiteral("kerf://atlas/rev22")));
        QVERIFY(links::isReviewLink(QStringLiteral("  KERF://atlas")));
        QVERIFY(!links::isReviewLink(QStringLiteral("https://atlas/rev22")));
        QVERIFY(!links::isReviewLink(QStringLiteral("/Users/ana/atlas")));

        links::ReviewLink link = parse(QStringLiteral("kerf://atlas/rev22@00:00:26:14"));
        QVERIFY2(link.valid, qPrintable(link.error));
        QCOMPARE(link.project, QStringLiteral("atlas"));
        QCOMPARE(link.revision, 22);
        QCOMPARE(link.timecode, QStringLiteral("00:00:26:14"));
        QVERIFY(std::abs(links::seconds(link, 25) - (26 + 14 / 25.0)) < 1e-9);

        link = parse(QStringLiteral("kerf://atlas/rev22?branch=main&head=4f2a9c1&remote=git%40github.com%3Akerf%2Fatlas.git"));
        QVERIFY(link.valid);
        QCOMPARE(link.revision, 22);
        QVERIFY(link.timecode.isEmpty());
        QVERIFY(std::isnan(links::seconds(link, 25)));
        QCOMPARE(link.branch, QStringLiteral("main"));
        QCOMPARE(link.head, QStringLiteral("4f2a9c1"));
        QCOMPARE(link.remote, QStringLiteral("git@github.com:kerf/atlas.git"));

        // A percent-encoded name, a drop-frame timecode, the project alone.
        link = parse(QStringLiteral("kerf://atlas%20cut/rev3@00:01:00;02"));
        QVERIFY(link.valid);
        QCOMPARE(link.project, QStringLiteral("atlas cut"));
        QCOMPARE(link.timecode, QStringLiteral("00:01:00;02"));
        link = parse(QStringLiteral("kerf://atlas"));
        QVERIFY(link.valid);
        QCOMPARE(link.revision, -1);

        // What is not a link says why.
        for (const char *bad : {"kerf://", "kerf:///rev2", "kerf://atlas/revision", "kerf://atlas/rev2@26:14", "http://atlas/rev2"}) {
            const links::ReviewLink refused = parse(QString::fromLatin1(bad));
            QVERIFY2(!refused.valid, bad);
            QVERIFY2(!refused.error.isEmpty(), bad);
        }
        QCOMPARE(links::slug(QStringLiteral(" Atlas Cut ")), QStringLiteral("atlas-cut"));
    }

    // A link names a project by its folder's slug; one this Mac does not
    // know, or two that share the name, are refused with words to say so.
    void linksFindTheirProjectOrSayWhyNot() {
        QTemporaryDir root;
        const QString atlas = makeProject(root, QStringLiteral("atlas"));
        const QString cut = makeProject(root, QStringLiteral("Atlas Cut"));
        QString error;
        QCOMPARE(links::resolve(QStringLiteral("atlas"), {atlas, cut}, &error), atlas);
        QCOMPARE(links::resolve(QStringLiteral("atlas-cut"), {atlas, cut + QStringLiteral("/")}, &error), cut);
        QVERIFY(links::resolve(QStringLiteral("harbour"), {atlas, cut}, &error).isEmpty());
        QVERIFY2(error.contains(QStringLiteral("No project named “harbour”")), qPrintable(error));
        QTemporaryDir elsewhere;
        const QString twin = makeProject(elsewhere, QStringLiteral("atlas"));
        QVERIFY(links::resolve(QStringLiteral("atlas"), {atlas, twin}, &error).isEmpty());
        QVERIFY2(error.contains(QStringLiteral("More than one project")), qPrintable(error));
    }

    // The router raises the editor that has the project open and asks it to
    // seek; a recent project without an editor is opened; an unknown one is
    // refused (shown to the user, quiet here). macOS hands a clicked link
    // over as a QFileOpenEvent to the application.
    void routerRaisesOrOpensTheLinkedProject() {
        QTemporaryDir root;
        const QString atlas = makeProject(root, QStringLiteral("atlas"));
        const QString harbour = makeProject(root, QStringLiteral("harbour"));
        auto open = std::make_unique<MainWindow>(atlas, QString());
        open->setConnection(QStringLiteral("ready"));
        open->applySnapshot(program(30.0, 22));
        QList<MainWindow *> windows{open.get()};
        std::unique_ptr<MainWindow> opened;
        QStringList openedFolders;
        links::Router router(
            [&](const QString &directory) {
                openedFolders << directory;
                opened = std::make_unique<MainWindow>(directory, QString());
                windows << opened.get();
                return opened.get();
            },
            [&] { return windows; });
        router.setQuiet(true);
        router.setKnownProjects([&] { return QStringList{atlas, harbour}; });
        QSignalSpy refused(&router, &links::Router::refused);

        // The open editor: raised, seeking to 00:00:26:14 at its 24 fps.
        QCOMPARE(router.follow(QStringLiteral("kerf://atlas/rev22@00:00:26:12")), open.get());
        QVERIFY(openedFolders.isEmpty());
        QVERIFY(!open->linkPending());
        QVERIFY(open->isVisible());
        QVERIFY2(std::abs(open->bus()->playhead() - (26 + 12 / 24.0)) < 1e-6, qPrintable(QString::number(open->bus()->playhead())));

        // A recent project with no editor: opened, the seek held until it
        // is connected and its programme is known.
        MainWindow *other = router.follow(QStringLiteral("kerf://harbour/rev4@00:00:02:00"));
        QCOMPARE(openedFolders, QStringList{harbour});
        QCOMPARE(other, opened.get());
        QVERIFY(other->linkPending());
        other->setConnection(QStringLiteral("ready"));
        other->applySnapshot(program(10.0, 4));
        QVERIFY(!other->linkPending());
        QVERIFY(std::abs(other->bus()->playhead() - 2.0) < 1e-6);
        QCOMPARE(refused.size(), 0);

        // Unknown, gone or not a link: refused, nothing opened.
        QVERIFY(!router.follow(QStringLiteral("kerf://lighthouse/rev1")));
        QVERIFY(router.lastError().contains(QStringLiteral("No project named “lighthouse”")));
        QFile::remove(QDir(harbour).filePath(QStringLiteral("project.kerf")));
        windows.removeAll(opened.get());
        QVERIFY(!router.follow(QStringLiteral("kerf://harbour/rev4")));
        QVERIFY2(router.lastError().contains(QStringLiteral("no longer at")), qPrintable(router.lastError()));
        QVERIFY(!router.follow(QStringLiteral("kerf://atlas/version-2")));
        QCOMPARE(refused.size(), 3);
        QCOMPARE(openedFolders.size(), 1);

        // macOS: the application's QFileOpenEvent with the URL.
        qApp->installEventFilter(&router);
        QFileOpenEvent event(QUrl(QStringLiteral("kerf://atlas/rev22@00:00:01:00")));
        QCoreApplication::sendEvent(qApp, &event);
        qApp->removeEventFilter(&router);
        QVERIFY(std::abs(open->bus()->playhead() - 1.0) < 1e-6);
        // A plain file is not the router's.
        QFileOpenEvent file(atlas + QStringLiteral("/project.kerf"));
        qApp->installEventFilter(&router);
        QCoreApplication::sendEvent(qApp, &file);
        qApp->removeEventFilter(&router);
        QCOMPARE(refused.size(), 3);
        open->close();
        if (opened) opened->close();
    }

    // Every link the editor writes names the folder's slug, a host QUrl
    // reads, so it survives macOS (which hands the click over as a
    // QFileOpenEvent built from a QUrl) and finds a project that is not
    // open: a folder named apart from its document, one with a space, one
    // with a letter outside ASCII. A kerf:// URL QUrl cannot read is
    // refused aloud.
    void linksSurviveTheFileOpenEventsQUrl() {
        QTemporaryDir root;
        const QString atlas = makeProject(root, QStringLiteral("atlas"));
        const QString cut = makeProject(root, QStringLiteral("Atlas Cut"));
        const QString cafe = makeProject(root, QStringLiteral("Café Cut"));
        QCOMPARE(links::slug(QStringLiteral("Café Cut")), QStringLiteral("café-cut"));
        // macOS hands a folder name over decomposed; the slug is the same.
        QCOMPARE(links::slug(QStringLiteral("Cafe\u0301 Cut")), QStringLiteral("café-cut"));
        QCOMPARE(links::slug(QStringLiteral("Atlas (v2) _final_")), QStringLiteral("atlas-v2-final"));
        QVERIFY(links::slug(QStringLiteral(" -- ")).isEmpty());
        QVERIFY(links::slug(QString(200, QLatin1Char('a'))).size() <= 63);

        // The comments' link: the folder, not the document's "Harbor Teaser".
        QString link;
        {
            MainWindow window{atlas, QString()};
            QJsonObject snapshot = program(30.0, 22);
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("name"), QStringLiteral("Harbor Teaser"));
            snapshot.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(snapshot);
            QCOMPARE(window.m_projectName, QStringLiteral("Harbor Teaser"));
            link = window.m_reviewWiring->reviewLink(1.0);
            QCOMPARE(link, links::make(atlas, 22, 1.0, window.m_fps));
            QVERIFY2(link.startsWith(QStringLiteral("kerf://atlas/rev22@")), qPrintable(link));
            window.close();
        }
        QVERIFY(links::make(QString(), 22).isEmpty());
        QVERIFY(links::make(atlas, 0).isEmpty());

        std::vector<std::unique_ptr<MainWindow>> opened;
        QStringList openedFolders;
        links::Router router(
            [&](const QString &directory) {
                openedFolders << directory;
                opened.push_back(std::make_unique<MainWindow>(directory, QString()));
                return opened.back().get();
            },
            [&] {
                QList<MainWindow *> windows;
                for (const auto &window : opened) windows << window.get();
                return windows;
            });
        router.setQuiet(true);
        router.setKnownProjects([&] { return QStringList{atlas, cut, cafe}; });
        QSignalSpy refused(&router, &links::Router::refused);
        qApp->installEventFilter(&router);
        const auto click = [](const QString &text) {
            QFileOpenEvent event{QUrl(text)};
            QCoreApplication::sendEvent(qApp, &event);
        };
        click(link);
        click(links::make(cut, 4, 2.0, 25.0));
        const QString cafeLink = links::make(cafe, 7);
        QCOMPARE(cafeLink, QStringLiteral("kerf://café-cut/rev7"));
        // The event's QUrl carries that host in ASCII.
        QVERIFY(QUrl(cafeLink).host(QUrl::FullyEncoded).startsWith(QStringLiteral("xn--")));
        click(cafeLink);
        QVERIFY2(refused.isEmpty(), qPrintable(router.lastError()));
        QCOMPARE(openedFolders, (QStringList{atlas, cut, cafe}));
        for (const auto &window : opened) QVERIFY(window->linkPending());

        // Read as QUrl writes it back, too.
        QCOMPARE(links::fromUrl(QUrl(cafeLink)), cafeLink);
        QCOMPARE(links::parse(QStringLiteral("kerf://xn--caf-cut-dya/rev7")).project, QStringLiteral("café-cut"));
        QVERIFY(links::fromUrl(QUrl(QStringLiteral("https://atlas/rev2"))).isEmpty());

        // A name QUrl cannot read as a host (the percent-encoded form older
        // links used): refused with words, not dropped.
        click(QStringLiteral("kerf://Atlas%20Cut/rev22@00:00:01:00"));
        QCOMPARE(refused.size(), 1);
        QVERIFY2(router.lastError().contains(QStringLiteral("could not be read")), qPrintable(router.lastError()));
        QCOMPARE(openedFolders.size(), 3);
        qApp->removeEventFilter(&router);
        for (const auto &window : opened) window->close();
    }

    // The editor answers to its folder's and its document's names, and a
    // link for another revision is noted, not hidden.
    void editorSaysWhenALinkNamesAnotherRevision() {
        QTemporaryDir root;
        const QString atlas = makeProject(root, QStringLiteral("Atlas Cut"));
        MainWindow window{atlas, QString()};
        QVERIFY(window.projectSlugs().contains(QStringLiteral("atlas-cut")));
        window.followLink(20, QStringLiteral("00:00:03:00"));
        QVERIFY(window.linkPending());
        // Not before it is connected.
        window.applySnapshot(program(30.0, 22));
        QVERIFY(window.linkPending());
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(program(30.0, 22));
        QVERIFY(!window.linkPending());
        QVERIFY(std::abs(window.bus()->playhead() - 3.0) < 1e-6);
        QVERIFY2(window.m_activityLog.join(QLatin1Char('\n')).contains(QStringLiteral("The review link is for rev 20; the project is at rev 22")),
                 qPrintable(window.m_activityLog.join(QLatin1Char('\n'))));
    }

    // The app bundle claims the kerf scheme (CFBundleURLTypes).
    void bundleRegistersTheKerfScheme() {
        QFile plist(QStringLiteral(KERF_QT_SOURCE_DIR "/Info.plist.in"));
        QVERIFY2(plist.open(QIODevice::ReadOnly), qPrintable(plist.fileName()));
        const QString text = QString::fromUtf8(plist.readAll());
        QVERIFY(text.contains(QStringLiteral("<key>CFBundleURLTypes</key>")));
        QVERIFY(text.contains(QStringLiteral("<string>kerf</string>")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::LinkTest)

#include "link_test.moc"
