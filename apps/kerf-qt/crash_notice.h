#pragma once

// After a crash: the reports kerfd, the editor and the CLI leave in the crash
// folder (kerf-runtime's crash.rs), and which of them the person has not been
// told about yet. Nothing is sent anywhere; the notice offers the folder.

#include <QJsonObject>
#include <QList>
#include <QString>

class QSettings;
class QWidget;

namespace kerf {

struct CrashReport {
    QString path;
    QString component;
    QString message;
    qint64 atMs = 0;
};

// The folder and its reports, as kerf_crash_reports gives them, newest first.
QString crashFolder(const QJsonObject &listed);
QList<CrashReport> crashReports(const QJsonObject &listed);
// What kerf_crash_reports says now.
QJsonObject listCrashReports();

// The reports newer than the last ones told about, and remembering that these
// have been.
QList<CrashReport> unseenCrashReports(const QList<CrashReport> &reports, const QSettings &settings);
void markCrashReportsSeen(const QList<CrashReport> &reports, QSettings &settings);

// "Kerf quit unexpectedly…": what the notice says of these reports.
QString crashNoticeText(const QList<CrashReport> &reports);

// Tells of any reports not told about yet, offering their folder, and
// remembers them. Returns whether it told of any.
bool showCrashNotice(QWidget *parent, QSettings &settings);

// Reports each of Qt's fatal messages (qFatal, and Qt's own, such as a
// platform plugin that would not load) before Qt aborts, with the message:
// on Windows that abort is a fail-fast no handler in the process sees. Call
// after kerf_crash_install and before the application is made.
void reportFatalMessages();

} // namespace kerf
