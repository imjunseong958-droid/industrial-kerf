#include "update_notice.h"

#include "bridge.h"

#include <QApplication>
#include <QDesktopServices>
#include <QJsonDocument>
#include <QMessageBox>
#include <QPointer>
#include <QPushButton>
#include <QSettings>
#include <QThread>
#include <QUrl>

namespace kerf {

namespace {
const QString kEnabled = QStringLiteral("updates/check");
const QString kCheckedAt = QStringLiteral("updates/checkedAt");
const QString kSkipped = QStringLiteral("updates/skipped");

void offer(QWidget *parent, const UpdateOffer &found) {
    auto *box = new QMessageBox(QMessageBox::Information, QStringLiteral("A newer Kerf is out"),
                                QStringLiteral("Industrial Kerf %1 is available. Download it from its release page, "
                                               "and unpack it over this one or beside it.")
                                    .arg(found.version),
                                QMessageBox::NoButton, parent);
    box->setAttribute(Qt::WA_DeleteOnClose);
    QPushButton *download = box->addButton(QStringLiteral("Download"), QMessageBox::AcceptRole);
    QPushButton *skip = box->addButton(QStringLiteral("Skip This Version"), QMessageBox::RejectRole);
    box->addButton(QStringLiteral("Later"), QMessageBox::RejectRole);
    box->setDefaultButton(download);
    QObject::connect(box, &QMessageBox::buttonClicked, box, [download, skip, found](QAbstractButton *button) {
        if (button == download) QDesktopServices::openUrl(QUrl(found.url));
        if (button == skip) QSettings().setValue(kSkipped, found.version);
    });
    box->open();
}
} // namespace

bool shouldCheckForUpdates(const QSettings &settings, const QDateTime &now) {
    if (!settings.value(kEnabled, true).toBool()) return false;
    const QDateTime last = settings.value(kCheckedAt).toDateTime();
    return !last.isValid() || last.secsTo(now) >= 20 * 60 * 60;
}

std::optional<UpdateOffer> updateOffer(const QJsonObject &answer, const QSettings &settings) {
    if (!answer.value(QStringLiteral("newer")).toBool()) return std::nullopt;
    const QString version = answer.value(QStringLiteral("latest")).toString();
    const QString url = answer.value(QStringLiteral("url")).toString();
    if (version.isEmpty() || url.isEmpty() || settings.value(kSkipped).toString() == version) return std::nullopt;
    return UpdateOffer{version, url};
}

void startUpdateCheck(QWidget *parent) {
    {
        QSettings settings;
        if (!shouldCheckForUpdates(settings, QDateTime::currentDateTimeUtc())) return;
        // Counted when asked, so a failing network is not asked again all day.
        settings.setValue(kCheckedAt, QDateTime::currentDateTimeUtc());
    }
    QPointer<QWidget> owner(parent);
    QThread *thread = QThread::create([owner] {
        QJsonObject answer;
        if (char *raw = kerf_update_check()) {
            answer = QJsonDocument::fromJson(QByteArray(raw)).object();
            kerf_bridge_string_free(raw);
        }
        QMetaObject::invokeMethod(qApp, [owner, answer] {
            if (const std::optional<UpdateOffer> found = updateOffer(answer, QSettings())) offer(owner, *found);
        });
    });
    QObject::connect(thread, &QThread::finished, thread, &QObject::deleteLater);
    thread->start();
}

} // namespace kerf
