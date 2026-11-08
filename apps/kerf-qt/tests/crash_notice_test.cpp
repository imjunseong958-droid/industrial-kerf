// The editor tells of crash reports once each, and offers their folder.

#include "test_registry.h"

#include "crash_notice.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QSettings>
#include <QTemporaryDir>
#include <QTest>

namespace kerf {

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
