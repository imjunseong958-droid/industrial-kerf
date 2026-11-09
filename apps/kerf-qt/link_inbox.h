#pragma once

// On Windows and Linux every kerf:// link a person follows starts an editor
// process of its own, where macOS hands it to the running app as a
// QFileOpenEvent. So the first editor listens in an inbox the user's own
// processes alone can reach, and one started for links alone gives them to it
// and ends: the link opens in the editor already there, as on macOS. Not built
// on macOS.

#include <QObject>
#include <QString>
#include <QStringList>

class QLocalServer;
class QSettings;

namespace kerf::links {

// Where this user's editors meet, by an id their settings keep: a socket in
// the user's own runtime folder on Linux, a pipe on Windows. Empty where Linux
// gives no runtime folder, and then there is no inbox.
QString inboxName(QSettings &settings);

// Gives `links` to the editor listening at `name`. True once it said it has
// them; false when none answered within `timeoutMs`, and the caller then opens
// them itself. Blocks, so it is for a process that has nothing else to do yet.
bool handOff(const QString &name, const QStringList &links, int timeoutMs = 2000);

// Listens at `name` for links handed over and emits each one that is a
// kerf:// review link; anything else is dropped.
class Inbox final : public QObject {
    Q_OBJECT
public:
    explicit Inbox(QObject *parent = nullptr);
    ~Inbox() override;

    // False when another editor already answers at `name`, which keeps the
    // links, or when the name cannot be taken. A socket an editor that crashed
    // left behind answers nobody, and is taken over.
    bool listen(const QString &name);

signals:
    void linkReceived(const QString &link);

private:
    void accept();

    QLocalServer *m_server;
};

} // namespace kerf::links
