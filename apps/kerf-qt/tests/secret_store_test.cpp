// API keys live in the system keychain, move there from plain settings, and
// stay in the settings only where the keychain cannot be reached.

#include "test_registry.h"

#include "secret_store.h"

#include <QHash>
#include <QSettings>
#include <QTest>

namespace kerf {

namespace {
const QString kKey = QStringLiteral("agent/anthropic/apiKey");
} // namespace

class SecretStoreTest final : public QObject {
    Q_OBJECT
private slots:
    void init() {
        m_saved = secrets::call();
        m_kept.clear();
        m_reachable = true;
        secrets::call() = [this](const QJsonObject &request) -> QJsonObject {
            if (!m_reachable) return {{QStringLiteral("error"), QStringLiteral("no keychain")}};
            const QString name = request.value(QStringLiteral("name")).toString();
            const QString op = request.value(QStringLiteral("op")).toString();
            if (op == QStringLiteral("get"))
                return {{QStringLiteral("ok"), true},
                        {QStringLiteral("value"), m_kept.contains(name) ? QJsonValue(m_kept.value(name)) : QJsonValue()}};
            if (op == QStringLiteral("set")) m_kept.insert(name, request.value(QStringLiteral("value")).toString());
            if (op == QStringLiteral("delete")) m_kept.remove(name);
            return {{QStringLiteral("ok"), true}};
        };
        QSettings().remove(kKey);
    }
    void cleanup() {
        secrets::call() = m_saved;
        QSettings().remove(kKey);
    }

    void aSavedKeyGoesToTheKeychainNotTheSettings() {
        QSettings settings;
        secrets::saveKey(settings, kKey, QStringLiteral("sk-ant-new"));
        QCOMPARE(m_kept.value(kKey), QStringLiteral("sk-ant-new"));
        QVERIFY(!settings.contains(kKey));
        QCOMPARE(secrets::loadKey(settings, kKey), QStringLiteral("sk-ant-new"));
    }

    void aKeyInPlainSettingsMovesIntoTheKeychain() {
        QSettings settings;
        settings.setValue(kKey, QStringLiteral("sk-ant-old"));
        QCOMPARE(secrets::loadKey(settings, kKey), QStringLiteral("sk-ant-old"));
        QCOMPARE(m_kept.value(kKey), QStringLiteral("sk-ant-old"));
        QVERIFY(!settings.contains(kKey));
    }

    void withNoKeychainKeysStayInTheSettings() {
        m_reachable = false;
        QSettings settings;
        secrets::saveKey(settings, kKey, QStringLiteral("sk-ant-here"));
        QCOMPARE(settings.value(kKey).toString(), QStringLiteral("sk-ant-here"));
        QCOMPARE(secrets::loadKey(settings, kKey), QStringLiteral("sk-ant-here"));
        QVERIFY(m_kept.isEmpty());
    }

    // A key changed while the keychain refused it (locked, a denied prompt, or on
    // Windows one longer than Credential Manager holds) is the newest, though the
    // keychain still has the one before it.
    void aKeyTheKeychainRefusedWinsOverTheOneKeptBefore() {
        QSettings settings;
        secrets::saveKey(settings, kKey, QStringLiteral("sk-ant-old"));
        m_reachable = false;
        secrets::saveKey(settings, kKey, QStringLiteral("sk-ant-new"));
        QCOMPARE(m_kept.value(kKey), QStringLiteral("sk-ant-old"));
        m_reachable = true;
        QCOMPARE(secrets::loadKey(settings, kKey), QStringLiteral("sk-ant-new"));
        QCOMPARE(m_kept.value(kKey), QStringLiteral("sk-ant-new"));
        QVERIFY(!settings.contains(kKey));
        QCOMPARE(secrets::loadKey(settings, kKey), QStringLiteral("sk-ant-new"));
    }

    void anEmptyKeyIsForgottenAndTheFallbackReads() {
        QSettings settings;
        secrets::saveKey(settings, kKey, QStringLiteral("sk-ant-gone"));
        secrets::saveKey(settings, kKey, QString());
        QVERIFY(m_kept.isEmpty());
        QVERIFY(!settings.contains(kKey));
        QCOMPARE(secrets::loadKey(settings, kKey, QStringLiteral("from-env")), QStringLiteral("from-env"));
    }

private:
    secrets::Call m_saved;
    QHash<QString, QString> m_kept;
    bool m_reachable = true;
};

} // namespace kerf

KERF_TEST_CLASS(kerf::SecretStoreTest)
#include "secret_store_test.moc"
