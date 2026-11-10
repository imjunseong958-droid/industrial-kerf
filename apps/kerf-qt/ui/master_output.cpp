#include "master_output.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QSettings>

namespace kerf::deliver {

namespace {

// The key as it was at first (deliver/<folder name>/<key>), still read while
// the one by path has nothing.
QString legacySettingsKey(const QString &directory, const QString &key) {
    QString project = projectName(directory);
    if (project.isEmpty()) return {};
    project.replace(QLatin1Char('/'), QLatin1Char('-')).replace(QLatin1Char('\\'), QLatin1Char('-'));
    return QStringLiteral("deliver/%1/%2").arg(project, key);
}

// What the project keeps for its master: {folder?, name?, codec?}.
QJsonObject savedMaster(const QString &directory) {
    const QString settings = settingsKey(directory, QStringLiteral("outputs"));
    if (settings.isEmpty()) return {};
    const QSettings store;
    const QString legacy = legacySettingsKey(directory, QStringLiteral("outputs"));
    const QString text = store.contains(settings) ? store.value(settings).toString() : store.value(legacy).toString();
    return QJsonDocument::fromJson(text.toUtf8()).object().value(QStringLiteral("master")).toObject();
}

} // namespace

QString projectName(const QString &directory) {
    if (directory.isEmpty()) return {};
    return QFileInfo(QDir::cleanPath(directory)).fileName();
}

QString settingsKey(const QString &directory, const QString &key) {
    QString project = projectName(directory);
    if (project.isEmpty()) return {};
    // A key's '/' would open a group of its own.
    project.replace(QLatin1Char('/'), QLatin1Char('-')).replace(QLatin1Char('\\'), QLatin1Char('-'));
    // Keyed by the absolute project path, so two projects whose folders
    // share a name keep their own: the folder's name (readable) and a digest
    // of its whole path.
    const QString path = QDir::cleanPath(QFileInfo(directory).absoluteFilePath());
    const QString digest = QString::fromLatin1(QCryptographicHash::hash(path.toUtf8(), QCryptographicHash::Sha1).toHex().left(12));
    return QStringLiteral("deliver/%1~%2/%3").arg(project, digest, key);
}

QJsonObject masterCommand(const QString &directory) {
    const QString project = projectName(directory);
    const QJsonObject kept = savedMaster(directory);
    QString folder = project.isEmpty() ? QString() : QStringLiteral("deliveries/");
    if (kept.contains(QStringLiteral("folder"))) folder = kept.value(QStringLiteral("folder")).toString();
    QString name = project.isEmpty() ? QString() : QStringLiteral("%1_master").arg(project);
    if (const QString own = kept.value(QStringLiteral("name")).toString().trimmed(); !own.isEmpty()) name = own;
    const QString codec = kept.value(QStringLiteral("codec")).toString();
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("export")}, {QStringLiteral("quality"), QStringLiteral("master")}};
    // ProRes 422 HQ is a master's own codec and goes without saying.
    if (!codec.isEmpty() && codec != QStringLiteral("prores_hq")) command.insert(QStringLiteral("codec"), codec);
    if (!folder.isEmpty()) command.insert(QStringLiteral("output"), folder);
    if (!name.isEmpty()) command.insert(QStringLiteral("name"), name);
    return command;
}

} // namespace kerf::deliver
