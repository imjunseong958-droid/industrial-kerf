#include "link_inbox.h"

#include "review_link.h"

#include <QDir>
#include <QLocalServer>
#include <QLocalSocket>
#include <QSettings>
#include <QStandardPaths>
#include <QUuid>

#include <memory>

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#endif

namespace kerf::links {

namespace {

// A message is links, one to a line, then an empty line; the inbox answers
// "ok" once it has them.
constexpr qsizetype kMaxMessage = 64 * 1024;
const QByteArray kEnd = QByteArrayLiteral("\n\n");
const QByteArray kTaken = QByteArrayLiteral("ok\n");

bool answers(const QString &name) {
    QLocalSocket probe;
    probe.connectToServer(name);
    const bool answered = probe.waitForConnected(250);
    probe.abort();
    return answered;
}

} // namespace

QString inboxName(QSettings &settings) {
    // A pipe's name is seen by every user of a Windows machine, and a name
    // another user could work out they could take first: so it carries an id
    // only this user's settings hold.
    QString id = settings.value(QStringLiteral("links/inbox")).toString();
    if (id.isEmpty()) {
        id = QUuid::createUuid().toString(QUuid::Id128);
        settings.setValue(QStringLiteral("links/inbox"), id);
    }
    const QString name = QStringLiteral("industrial-kerf-links-") + id;
#ifdef Q_OS_WIN
    return name;
#else
    // Not the shared temporary folder, where another user could put a socket
    // first: without a runtime folder of the user's own there is no inbox.
    const QString runtime = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    return runtime.isEmpty() ? QString() : QDir(runtime).filePath(name);
#endif
}

bool handOff(const QString &name, const QStringList &links, int timeoutMs) {
    QByteArray message;
    for (const QString &link : links) {
        const QString line = link.trimmed();
        if (isReviewLink(line) && !line.contains(QLatin1Char('\n')) && !line.contains(QLatin1Char('\r')))
            message += line.toUtf8() + '\n';
    }
    if (name.isEmpty() || message.isEmpty()) return false;
    message += '\n';

    QLocalSocket socket;
    socket.connectToServer(name);
    if (!socket.waitForConnected(timeoutMs)) return false;
#ifdef Q_OS_WIN
    // Windows brings a window to the front only for the process the person
    // just started, so this one, which they did, lets the editor that takes
    // the links come forward.
    AllowSetForegroundWindow(ASFW_ANY);
#endif
    socket.write(message);
    if (!socket.waitForBytesWritten(timeoutMs)) return false;
    QByteArray answer;
    while (!answer.contains('\n')) {
        if (socket.bytesAvailable() == 0 && !socket.waitForReadyRead(timeoutMs)) break;
        answer += socket.readAll();
    }
    return answer == kTaken;
}

Inbox::Inbox(QObject *parent) : QObject(parent), m_server(new QLocalServer(this)) {
    // Only this user's processes may connect.
    m_server->setSocketOptions(QLocalServer::UserAccessOption);
    connect(m_server, &QLocalServer::newConnection, this, &Inbox::accept);
}

Inbox::~Inbox() = default;

bool Inbox::listen(const QString &name) {
    if (name.isEmpty() || answers(name)) return false;
    // On Linux a socket left by an editor that crashed answers nobody but is
    // still in the way.
    QLocalServer::removeServer(name);
    return m_server->listen(name);
}

void Inbox::accept() {
    while (QLocalSocket *socket = m_server->nextPendingConnection()) {
        connect(socket, &QLocalSocket::disconnected, socket, &QObject::deleteLater);
        auto received = std::make_shared<QByteArray>();
        connect(socket, &QLocalSocket::readyRead, this, [this, socket, received] {
            received->append(socket->readAll());
            if (received->size() > kMaxMessage) {
                socket->abort();
                return;
            }
            const qsizetype end = received->indexOf(kEnd);
            if (end < 0) return;
            QStringList links;
            for (const QByteArray &line : received->left(end).split('\n')) {
                const QString link = QString::fromUtf8(line).trimmed();
                if (isReviewLink(link)) links << link;
            }
            received->clear();
            // Answered first, so the process that handed them over is not left
            // waiting while a link is followed, which may ask the person
            // something; then followed from the event loop, apart from this
            // socket.
            socket->write(kTaken);
            socket->flush();
            socket->disconnectFromServer();
            QMetaObject::invokeMethod(
                this,
                [this, links] {
                    for (const QString &link : links) emit linkReceived(link);
                },
                Qt::QueuedConnection);
        });
    }
}

} // namespace kerf::links
