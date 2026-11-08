// Kerf asks GitHub at most once a day, offers only a newer release, and not
// one the person skipped.

#include "test_registry.h"

#include "update_notice.h"

#include <QSettings>
#include <QTest>

namespace kerf {

class UpdateNoticeTest final : public QObject {
    Q_OBJECT
private slots:
    void init() { QSettings().remove(QStringLiteral("updates")); }
    void cleanup() { QSettings().remove(QStringLiteral("updates")); }

    void asksAtMostOnceADayAndNotWhenTurnedOff() {
        QSettings settings;
        const QDateTime now = QDateTime::fromString(QStringLiteral("2026-11-08T12:00:00Z"), Qt::ISODate);
        QVERIFY(shouldCheckForUpdates(settings, now));
        settings.setValue(QStringLiteral("updates/checkedAt"), now.addSecs(-3600));
        QVERIFY(!shouldCheckForUpdates(settings, now));
        settings.setValue(QStringLiteral("updates/checkedAt"), now.addDays(-1));
        QVERIFY(shouldCheckForUpdates(settings, now));
        settings.setValue(QStringLiteral("updates/check"), false);
        QVERIFY(!shouldCheckForUpdates(settings, now));
    }

    void offersANewerReleaseUnlessSkipped() {
        QSettings settings;
        const QJsonObject newer{{QStringLiteral("current"), QStringLiteral("0.1.0")},
                                {QStringLiteral("latest"), QStringLiteral("0.2.0")},
                                {QStringLiteral("newer"), true},
                                {QStringLiteral("url"), QStringLiteral("https://example.com/v0.2.0")}};
        const std::optional<UpdateOffer> offered = updateOffer(newer, settings);
        QVERIFY(offered);
        QCOMPARE(offered->version, QStringLiteral("0.2.0"));
        QCOMPARE(offered->url, QStringLiteral("https://example.com/v0.2.0"));
        settings.setValue(QStringLiteral("updates/skipped"), QStringLiteral("0.2.0"));
        QVERIFY(!updateOffer(newer, settings));
        // Nothing newer, or no answer, offers nothing.
        QVERIFY(!updateOffer({{QStringLiteral("newer"), false}}, settings));
        QVERIFY(!updateOffer({{QStringLiteral("error"), QStringLiteral("offline")}}, settings));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::UpdateNoticeTest)
#include "update_notice_test.moc"
