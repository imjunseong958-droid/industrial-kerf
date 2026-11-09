// The editor tells of crash reports once each, and offers their folder.

#include "test_registry.h"

#include "bridge.h"
#include "crash_notice.h"

#include <QCoreApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QSettings>
#include <QTemporaryDir>
#include <QTest>

#include <cstdint>

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <stdlib.h>
#include <windows.h>
#endif

namespace kerf {

namespace test {

// What a child of the crash tests below does in place of testing: set up as
// the editor's main() does, then crash as KERF_QT_CRASH_ON_PURPOSE says.
int crashOnPurpose() {
#ifdef Q_OS_WIN
    // On a desktop Windows would otherwise ask whether to look for a solution.
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);
#endif
    kerf_crash_install();
    reportFatalMessages();
    const QByteArray how = qgetenv("KERF_QT_CRASH_ON_PURPOSE");
    if (how == "segfault") *reinterpret_cast<volatile int *>(std::uintptr_t{16}) = 1;
    if (how == "qfatal") qFatal("kerf-qt-tests: a fatal message on purpose");
    return 3;
}

} // namespace test

namespace {

// The reports a child of this test binary left, crashing as `how` says.
QList<CrashReport> reportsOfAChildThatCrashed(const QByteArray &how) {
    QTemporaryDir folder;
    QProcess child;
    QProcessEnvironment environment = QProcessEnvironment::systemEnvironment();
    environment.insert(QStringLiteral("KERF_QT_CRASH_ON_PURPOSE"), QString::fromLatin1(how));
    environment.insert(QStringLiteral("KERF_CRASH_DIR"), folder.path());
    child.setProcessEnvironment(environment);
    child.start(QCoreApplication::applicationFilePath(), {});
    if (!child.waitForFinished(60000)) {
        child.kill();
        qWarning("the child did not crash in a minute");
        return {};
    }
    if (child.exitStatus() == QProcess::NormalExit && child.exitCode() == 3) {
        qWarning("the child did not crash");
        return {};
    }
    // As the editor reads them when it next opens.
    qputenv("KERF_CRASH_DIR", folder.path().toLocal8Bit());
    const QList<CrashReport> reports = crashReports(listCrashReports());
    qunsetenv("KERF_CRASH_DIR");
    return reports;
}

} // namespace

class CrashNoticeTest final : public QObject {
    Q_OBJECT
private slots:
    void init() { QSettings().remove(QStringLiteral("crashes")); }
    void cleanup() { QSettings().remove(QStringLiteral("crashes")); }

    void readsWhatTheBridgeLists() {
        const QJsonObject listed{
            {QStringLiteral("folder"), QStringLiteral("/tmp/crashes")},
            {QStringLiteral("reports"),
             QJsonArray{QJsonObject{{QStringLiteral("path"), QStringLiteral("/tmp/crashes/kerfd-9-1.txt")},
                                    {QStringLiteral("component"), QStringLiteral("kerfd")},
                                    {QStringLiteral("atMs"), 1762600000123.0},
                                    {QStringLiteral("message"), QStringLiteral("boom")}}}}};
        QCOMPARE(crashFolder(listed), QStringLiteral("/tmp/crashes"));
        const QList<CrashReport> reports = crashReports(listed);
        QCOMPARE(reports.size(), 1);
        QCOMPARE(reports.first().component, QStringLiteral("kerfd"));
        QCOMPARE(reports.first().atMs, qint64(1762600000123));
        QCOMPARE(reports.first().message, QStringLiteral("boom"));
        QVERIFY(crashReports(QJsonObject{}).isEmpty());
    }

    void eachReportIsToldOfOnce() {
        QSettings settings;
        const QList<CrashReport> first{{QStringLiteral("a"), QStringLiteral("editor"), QStringLiteral("one"), 100}};
        QCOMPARE(unseenCrashReports(first, settings).size(), 1);
        markCrashReportsSeen(first, settings);
        QVERIFY(unseenCrashReports(first, settings).isEmpty());
        // A newer one is told of; the old one is not again.
        const QList<CrashReport> both{{QStringLiteral("b"), QStringLiteral("kerfd"), QStringLiteral("two"), 200}, first.first()};
        const QList<CrashReport> unseen = unseenCrashReports(both, settings);
        QCOMPARE(unseen.size(), 1);
        QCOMPARE(unseen.first().message, QStringLiteral("two"));
    }

    void theNoticeSaysWhichPartAndWhat() {
        QCOMPARE(crashNoticeText({}), QString());
        const QString one = crashNoticeText({{QStringLiteral("p"), QStringLiteral("kerfd"), QStringLiteral("index failed"), 1}});
        QVERIFY2(one.contains(QStringLiteral("kerfd")), qPrintable(one));
        QVERIFY2(one.contains(QStringLiteral("index failed")), qPrintable(one));
        QVERIFY2(one.contains(QStringLiteral("this computer only")), qPrintable(one));
        const QString two = crashNoticeText({{QStringLiteral("p"), QStringLiteral("editor"), QStringLiteral("x"), 2},
                                             {QStringLiteral("q"), QStringLiteral("kerfd"), QStringLiteral("y"), 1}});
        QVERIFY2(two.contains(QStringLiteral("2 times")), qPrintable(two));
        QVERIFY2(two.contains(QStringLiteral("the editor")), qPrintable(two));
    }

    // A crash in the editor's own C++, which no panic hook sees, is told of the
    // next time it opens.
    void aCrashInTheEditorsCppIsToldOfNextTime() {
        const QList<CrashReport> reports = reportsOfAChildThatCrashed("segfault");
        QCOMPARE(reports.size(), 1);
        QCOMPARE(reports.first().component, QStringLiteral("editor"));
        const QString message = reports.first().message;
        QVERIFY2(message.contains(QStringLiteral("SIGSEGV")) || message.contains(QStringLiteral("Access violation")),
                 qPrintable(message));
        QSettings settings;
        QCOMPARE(unseenCrashReports(reports, settings).size(), 1);
        QVERIFY(crashNoticeText(reports).contains(QStringLiteral("the editor")));
    }

    // Qt's own fatal errors end in an abort, a fail-fast on Windows that no
    // handler sees: the message is reported before it, once.
    void aFatalMessageFromQtIsToldOfWithTheMessage() {
        const QList<CrashReport> reports = reportsOfAChildThatCrashed("qfatal");
        QCOMPARE(reports.size(), 1);
        QCOMPARE(reports.first().message, QStringLiteral("kerf-qt-tests: a fatal message on purpose"));
        QVERIFY(crashNoticeText(reports).contains(QStringLiteral("a fatal message on purpose")));
    }

    // Nothing to tell: no dialog, nothing remembered.
    void noReportsShowNothing() {
        QTemporaryDir empty;
        qputenv("KERF_CRASH_DIR", empty.path().toLocal8Bit());
        QSettings settings;
        QVERIFY(!showCrashNotice(nullptr, settings));
        QVERIFY(!settings.contains(QStringLiteral("crashes/seenMs")));
        qunsetenv("KERF_CRASH_DIR");
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::CrashNoticeTest)
#include "crash_notice_test.moc"
