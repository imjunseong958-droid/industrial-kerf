#include "crash_notice.h"

#include "bridge.h"

#include <QDesktopServices>
#include <QJsonArray>
#include <QJsonDocument>
#include <QMessageBox>
#include <QPushButton>
#include <QSettings>
#include <QUrl>
#include <QtGlobal>

namespace kerf {

namespace {
const QString kSeenKey = QStringLiteral("crashes/seenMs");

// Whatever handled Qt's messages before reportFatalMessages: Qt's own, which
// writes them to stderr (or the debugger), unless something else came first.
QtMessageHandler earlierMessageHandler = nullptr;

void reportFatalMessage(QtMsgType type, const QMessageLogContext &context, const QString &message) {
    if (type == QtFatalMsg) kerf_crash_fatal(message.toUtf8().constData());
    if (earlierMessageHandler) earlierMessageHandler(type, context, message);
}

QString componentName(const QString &component) {
    if (component == QStringLiteral("kerfd")) return QStringLiteral("Kerf's project service (kerfd)");
    if (component == QStringLiteral("kerf")) return QStringLiteral("the kerf command");
    return QStringLiteral("the editor");
}
} // namespace

QString crashFolder(const QJsonObject &listed) { return listed.value(QStringLiteral("folder")).toString(); }

QList<CrashReport> crashReports(const QJsonObject &listed) {
    QList<CrashReport> reports;
    for (const QJsonValue &value : listed.value(QStringLiteral("reports")).toArray()) {
        const QJsonObject report = value.toObject();
        reports.append({report.value(QStringLiteral("path")).toString(), report.value(QStringLiteral("component")).toString(),
                        report.value(QStringLiteral("message")).toString(),
                        static_cast<qint64>(report.value(QStringLiteral("atMs")).toDouble())});
    }
    return reports;
}

QJsonObject listCrashReports() {
    char *raw = kerf_crash_reports();
    if (!raw) return {};
    const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
    kerf_bridge_string_free(raw);
    return document.object();
}

QList<CrashReport> unseenCrashReports(const QList<CrashReport> &reports, const QSettings &settings) {
    const qint64 seen = settings.value(kSeenKey, 0).toLongLong();
    QList<CrashReport> unseen;
    for (const CrashReport &report : reports)
        if (report.atMs > seen) unseen.append(report);
    return unseen;
}

void markCrashReportsSeen(const QList<CrashReport> &reports, QSettings &settings) {
    qint64 newest = settings.value(kSeenKey, 0).toLongLong();
    for (const CrashReport &report : reports) newest = qMax(newest, report.atMs);
    settings.setValue(kSeenKey, newest);
}

QString crashNoticeText(const QList<CrashReport> &reports) {
    if (reports.isEmpty()) return {};
    const CrashReport &latest = reports.first();
    QString text = reports.size() == 1
                       ? QStringLiteral("Kerf stopped unexpectedly: %1 hit an error it could not recover from.")
                             .arg(componentName(latest.component))
                       : QStringLiteral("Kerf stopped unexpectedly %1 times since it last said so. The latest was in %2.")
                             .arg(reports.size())
                             .arg(componentName(latest.component));
    if (!latest.message.isEmpty()) text += QStringLiteral("\n\n“%1”").arg(latest.message);
    text += QStringLiteral("\n\nA report of each is in the crash folder, on this computer only. Attaching one to a bug "
                           "report shows where it happened.");
    return text;
}

void reportFatalMessages() { earlierMessageHandler = qInstallMessageHandler(reportFatalMessage); }

bool showCrashNotice(QWidget *parent, QSettings &settings) {
    const QJsonObject listed = listCrashReports();
    const QList<CrashReport> unseen = unseenCrashReports(crashReports(listed), settings);
    if (unseen.isEmpty()) return false;
    markCrashReportsSeen(unseen, settings);
    QMessageBox box(QMessageBox::Warning, QStringLiteral("Kerf stopped unexpectedly"), crashNoticeText(unseen),
                    QMessageBox::Close, parent);
    QPushButton *open = box.addButton(QStringLiteral("Show Crash Folder"), QMessageBox::ActionRole);
    box.setDefaultButton(QMessageBox::Close);
    box.exec();
    if (box.clickedButton() == open) QDesktopServices::openUrl(QUrl::fromLocalFile(crashFolder(listed)));
    return true;
}

} // namespace kerf
