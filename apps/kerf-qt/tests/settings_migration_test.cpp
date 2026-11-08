// Settings from an older Kerf come up to this one's version once, and
// settings from a newer one are left alone.

#include "test_registry.h"

#include "settings_migration.h"

#include <QSettings>
#include <QTest>

namespace kerf {

namespace {
const QStringList kRetired{QStringLiteral("workspace/current"),    QStringLiteral("workspace/names"),
                           QStringLiteral("workspace/changes/Review"), QStringLiteral("workspace/stage/Review"),
                           QStringLiteral("viewer/chatOpen"),      QStringLiteral("viewer/chatWidth"),
                           QStringLiteral("viewer/devicePreview"), QStringLiteral("viewer/laptopPreview"),
                           QStringLiteral("editor/inspectorFollowsSelection")};
} // namespace

class SettingsMigrationTest final : public QObject {
    Q_OBJECT
private slots:
    void init() { clear(); }
    void cleanup() { clear(); }

    void retiredKeysGoAndTheVersionIsWritten() {
        QSettings settings;
        for (const QString &key : kRetired) settings.setValue(key, QStringLiteral("old"));
        settings.setValue(QStringLiteral("viewer/safeZones"), true);
        QCOMPARE(migrateSettings(settings), 0);
        for (const QString &key : kRetired) QVERIFY2(!settings.contains(key), qPrintable(key));
        // What is still read stays.
        QCOMPARE(settings.value(QStringLiteral("viewer/safeZones")).toBool(), true);
        QCOMPARE(settings.value(QStringLiteral("settings/version")).toInt(), kSettingsVersion);
        // Once only.
        settings.setValue(QStringLiteral("viewer/chatOpen"), true);
        QCOMPARE(migrateSettings(settings), kSettingsVersion);
        QVERIFY(settings.contains(QStringLiteral("viewer/chatOpen")));
    }

    void aRetiredDefaultModelGivesWayAndAChosenOneStays() {
        QSettings settings;
        for (const QString &retired : {QStringLiteral("claude-sonnet-4-20250514"), QStringLiteral("claude-3-5-sonnet-latest")}) {
            settings.remove(QStringLiteral("settings"));
            settings.setValue(QStringLiteral("agent/anthropic/model"), retired);
            migrateSettings(settings);
            QVERIFY2(!settings.contains(QStringLiteral("agent/anthropic/model")), qPrintable(retired));
        }
        settings.remove(QStringLiteral("settings"));
        settings.setValue(QStringLiteral("agent/anthropic/model"), QStringLiteral("claude-sonnet-5-5"));
        migrateSettings(settings);
        QCOMPARE(settings.value(QStringLiteral("agent/anthropic/model")).toString(), QStringLiteral("claude-sonnet-5-5"));
    }

    void aNewerKerfsSettingsAreLeftAlone() {
        QSettings settings;
        settings.setValue(QStringLiteral("settings/version"), kSettingsVersion + 1);
        settings.setValue(QStringLiteral("viewer/chatOpen"), true);
        QCOMPARE(migrateSettings(settings), kSettingsVersion + 1);
        QCOMPARE(settings.value(QStringLiteral("settings/version")).toInt(), kSettingsVersion + 1);
        QVERIFY(settings.contains(QStringLiteral("viewer/chatOpen")));
    }

private:
    static void clear() {
        QSettings settings;
        for (const QString &group : {QStringLiteral("settings"), QStringLiteral("workspace"), QStringLiteral("viewer"),
                                     QStringLiteral("editor/inspectorFollowsSelection"), QStringLiteral("agent/anthropic/model")})
            settings.remove(group);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::SettingsMigrationTest)
#include "settings_migration_test.moc"
