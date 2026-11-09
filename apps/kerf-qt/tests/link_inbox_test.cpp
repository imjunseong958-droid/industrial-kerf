// On Windows and Linux an editor started for a kerf:// link hands it to the
// one already open, through an inbox only the user's own processes reach, and
// opens it itself only when no editor took it. Not built on macOS, which gives
// a link to the running app.

#include "test_registry.h"

#include "link_inbox.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QLocalSocket>
#include <QSettings>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QTest>
#include <QThread>
#include <QUuid>

#include <functional>

namespace kerf {

namespace {

QString uniqueName() { return QStringLiteral("kerf-inbox-test-") + QUuid::createUuid().toString(QUuid::Id128); }

// Runs `work` on a thread of its own, as another process would, while this
// thread's event loop serves the inbox.
void elsewhere(const std::function<void()> &work) {
    QThread *thread = QThread::create(work);
    thread->start();
    while (!thread->wait(QDeadlineTimer(10))) QCoreApplication::processEvents(QEventLoop::AllEvents, 10);
    delete thread;
}

} // namespace

class LinkInboxTest final : public QObject {
    Q_OBJECT
private slots:
    void aLinkHandedOverReachesTheOpenEditor() {
        links::Inbox inbox;
        const QString name = uniqueName();
        QVERIFY(inbox.listen(name));
        QSignalSpy received(&inbox, &links::Inbox::linkReceived);
        bool taken = false;
        elsewhere([&] {
            taken = links::handOff(name, {QStringLiteral("kerf://my-film/rev3@00:00:01:00"),
                                          QStringLiteral(" kerf://my-film/rev4 ")});
        });
        QVERIFY(taken);
        QTRY_COMPARE(received.count(), 2);
        QCOMPARE(received.at(0).at(0).toString(), QStringLiteral("kerf://my-film/rev3@00:00:01:00"));
        QCOMPARE(received.at(1).at(0).toString(), QStringLiteral("kerf://my-film/rev4"));
    }

    void withNoEditorOpenTheLinkIsNotTaken() {
        QVERIFY(!links::handOff(uniqueName(), {QStringLiteral("kerf://my-film/rev3")}, 500));
        // Nor is anything that is not a review link, so there is nothing to send.
        links::Inbox inbox;
        const QString name = uniqueName();
        QVERIFY(inbox.listen(name));
        QVERIFY(!links::handOff(name, {QStringLiteral("file:///etc/passwd")}, 500));
        QVERIFY(!links::handOff(QString(), {QStringLiteral("kerf://my-film/rev3")}, 500));
    }

    // What another process writes to the inbox is followed only if it is a
    // review link.
    void onlyReviewLinksAreFollowed() {
        links::Inbox inbox;
        const QString name = uniqueName();
        QVERIFY(inbox.listen(name));
        QSignalSpy received(&inbox, &links::Inbox::linkReceived);
        QByteArray answer;
        elsewhere([&] {
            QLocalSocket socket;
            socket.connectToServer(name);
            if (!socket.waitForConnected(2000)) return;
            socket.write("file:///etc/passwd\n--settings-dir\nkerf://my-film/rev2\n\n");
            socket.waitForBytesWritten(2000);
            while (!answer.contains('\n') && socket.waitForReadyRead(2000)) answer += socket.readAll();
        });
        QCOMPARE(answer, QByteArrayLiteral("ok\n"));
        QTRY_COMPARE(received.count(), 1);
        QCOMPARE(received.at(0).at(0).toString(), QStringLiteral("kerf://my-film/rev2"));
    }

    void aSecondEditorLeavesTheInboxToTheFirst() {
        links::Inbox first;
        links::Inbox second;
        const QString name = uniqueName();
        QVERIFY(first.listen(name));
        QVERIFY(!second.listen(name));
        QSignalSpy toFirst(&first, &links::Inbox::linkReceived);
        bool taken = false;
        elsewhere([&] { taken = links::handOff(name, {QStringLiteral("kerf://my-film/rev5")}); });
        QVERIFY(taken);
        QTRY_COMPARE(toFirst.count(), 1);
    }

#ifndef Q_OS_WIN
    // A crashed editor's socket file stays where the next editor would listen.
    void aSocketLeftByACrashIsTakenOver() {
        const QString name = uniqueName();
        QFile left(QDir(QDir::tempPath()).filePath(name));
        QVERIFY(left.open(QIODevice::WriteOnly));
        left.close();
        links::Inbox inbox;
        QVERIFY(inbox.listen(name));
        QSignalSpy received(&inbox, &links::Inbox::linkReceived);
        bool taken = false;
        elsewhere([&] { taken = links::handOff(name, {QStringLiteral("kerf://my-film/rev6")}); });
        QVERIFY(taken);
        QTRY_COMPARE(received.count(), 1);
    }
#endif

    // The name carries an id this user's settings keep, so another user can
    // neither work it out nor take it first; on Linux it is in the user's own
    // runtime folder, not the shared temporary one.
    void theInboxIsNamedByAnIdTheSettingsKeep() {
        QSettings settings;
        settings.remove(QStringLiteral("links/inbox"));
        const QString name = links::inboxName(settings);
        QVERIFY(!name.isEmpty());
        const QString id = settings.value(QStringLiteral("links/inbox")).toString();
        QCOMPARE(id.size(), 32);
        QVERIFY(name.endsWith(QStringLiteral("industrial-kerf-links-") + id));
        QCOMPARE(links::inboxName(settings), name);
#ifndef Q_OS_WIN
        QCOMPARE(QFileInfo(name).absolutePath(),
                 QFileInfo(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation)).absoluteFilePath());
#endif
        settings.remove(QStringLiteral("links/inbox"));
        QVERIFY(links::inboxName(settings) != name);
        settings.remove(QStringLiteral("links/inbox"));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::LinkInboxTest)
#include "link_inbox_test.moc"
