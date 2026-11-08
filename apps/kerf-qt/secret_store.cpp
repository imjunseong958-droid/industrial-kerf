#include "secret_store.h"

#include "bridge.h"

#include <QJsonDocument>
#include <QSettings>

namespace kerf::secrets {

namespace {
QJsonObject bridgeCall(const QJsonObject &request) {
    const QByteArray json = QJsonDocument(request).toJson(QJsonDocument::Compact);
    char *raw = kerf_secret(json.constData());
    if (!raw) return {{QStringLiteral("error"), QStringLiteral("Kerf did not answer.")}};
    const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
    kerf_bridge_string_free(raw);
    return document.object();
}

struct Found {
    bool reachable = false;
    bool found = false;
    QString value;
};

Found read(const QString &key) {
    const QJsonObject answer = call()({{QStringLiteral("op"), QStringLiteral("get")}, {QStringLiteral("name"), key}});
    if (!answer.value(QStringLiteral("ok")).toBool()) return {};
    const QJsonValue value = answer.value(QStringLiteral("value"));
    return {true, value.isString(), value.toString()};
}

bool write(const QString &key, const QString &value) {
    return call()({{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("name"), key}, {QStringLiteral("value"), value}})
        .value(QStringLiteral("ok"))
        .toBool();
}

bool forget(const QString &key) {
    return call()({{QStringLiteral("op"), QStringLiteral("delete")}, {QStringLiteral("name"), key}}).value(QStringLiteral("ok")).toBool();
}
} // namespace

Call &call() {
    static Call current = bridgeCall;
    return current;
}

QString loadKey(QSettings &settings, const QString &key, const QString &fallback) {
    const Found kept = read(key);
    if (kept.found) {
        // A copy left in plain settings has no business there any more.
        settings.remove(key);
        return kept.value;
    }
    if (settings.contains(key)) {
        const QString value = settings.value(key).toString();
        if (kept.reachable && !value.isEmpty() && write(key, value)) settings.remove(key);
        return value;
    }
    return fallback;
}

void saveKey(QSettings &settings, const QString &key, const QString &value) {
    if (value.isEmpty()) {
        forget(key);
        settings.remove(key);
        return;
    }
    if (write(key, value))
        settings.remove(key);
    else
        settings.setValue(key, value);
}

} // namespace kerf::secrets
