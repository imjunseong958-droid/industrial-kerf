#pragma once

// Customise Keyboard… (keyboard.customise, ⌥⌘K; mockup-spec 9 and 14, the
// Keyboard menu): one's own shortcuts over the keymap in use.
//
//   KeymapCustomiser  keeps them in the editor's settings
//                     (commands/custom/<keymap>/<command id> = the keys in
//                     Qt's portable text, an empty list for none), applies
//                     them to the registry's actions whenever the registry
//                     loads a keymap (its shortcutsChanged), and changes them
//                     live: every menu, tooltip and panel that follows the
//                     registry follows them.
//   KeymapEditor      a flat dialog: search, the commands with where they
//                     act and their shortcut, a recorder for the chosen one,
//                     a warning when a key is taken (in the same panel or
//                     window-wide), and Reset to default / Reset all.
//
// The commands listed are the registry's (kerf_commands_catalog) that a key
// can run: those in a menu, those with a key in some keymap, and the
// frontend's own (bin.search, source.play, composer.draw, ...). Engine
// commands that need parameters a key cannot give (clip.move) are not.

#include "test_friends.h"

#include <QDialog>
#include <QHash>
#include <QKeySequence>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>

class QKeySequenceEdit;
class QLabel;
class QLineEdit;
class QListWidget;

namespace kerf {

class CommandRegistry;

namespace ui {
class Button;
class GhostButton;
} // namespace ui

class KeymapCustomiser final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS

public:
    struct Command {
        QString id;
        QString label;
        QString where;  // the menu, else the panel it acts in
        QString scope;  // window, timeline, source (the registry's)
        QList<QKeySequence> defaults; // the keymap's own
    };

    explicit KeymapCustomiser(CommandRegistry *registry, QObject *parent = nullptr);
    CommandRegistry *registry() const { return m_registry; }
    QString keymap() const { return m_keymap; }
    // The settings group one's own keys for `keymap` live in.
    static QString settingsGroup(const QString &keymap);

    // The commands a key can be given to, in the registry's order.
    QList<Command> commands() const { return m_commands; }
    Command command(const QString &id) const;
    bool has(const QString &id) const { return m_index.contains(id); }
    // The keys the command answers to now, and the keymap's own.
    QList<QKeySequence> keys(const QString &id) const;
    QList<QKeySequence> defaults(const QString &id) const;
    bool isCustom(const QString &id) const;
    // The commands `key` would collide with if `id` took it: those bound to
    // it in the same panel, or window-wide (a window key collides with
    // every panel's).
    QStringList conflicts(const QString &id, const QKeySequence &key) const;
    static bool scopesOverlap(const QString &a, const QString &b);

    // `id` answers to `key` alone; the commands it collided with lose it.
    void assign(const QString &id, const QKeySequence &key);
    // `id` answers to no key.
    void clear(const QString &id);
    // `id` back to the keymap's keys, or every command.
    void reset(const QString &id);
    void resetAll();
    // Applies the stored keys to the registry's actions (after it loads).
    void apply();

signals:
    void changed();

private:
    void load();
    void store(const QString &id, const QList<QKeySequence> &keys);
    void setKeys(const QString &id, const QList<QKeySequence> &keys);
    void announce();

    QPointer<CommandRegistry> m_registry;
    QString m_keymap;
    QList<Command> m_commands;
    QHash<QString, int> m_index;
    bool m_applying = false;
};

class KeymapEditor final : public QDialog {
    Q_OBJECT
    KERF_TEST_FRIENDS

public:
    static constexpr int kWidth = 560;
    static constexpr int kHeight = 600;
    static constexpr int kRow = 28;

    explicit KeymapEditor(KeymapCustomiser *customiser, QWidget *parent = nullptr);
    KeymapCustomiser *customiser() const { return m_customiser; }

    // Search: the commands whose name, menu or key has every word.
    void setFilter(const QString &text);
    QStringList shownCommands() const;
    void selectCommand(const QString &id);
    QString selectedCommand() const;
    // What a row shows on its right: the keys, "" for none.
    QString keyText(const QString &id) const;

    // A key recorded for the chosen command: it takes it now, or, when the
    // key is taken, the warning asks first (confirmConflict / cancelConflict).
    void record(const QKeySequence &key);
    bool conflictPending() const { return !m_pendingKey.isEmpty(); }
    QString warning() const;
    void confirmConflict();
    void cancelConflict();

    QLineEdit *searchField() const { return m_search; }
    QListWidget *list() const { return m_list; }
    QKeySequenceEdit *recorder() const { return m_recorder; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void rebuild();
    void refreshRows();
    void syncEditor();

    KeymapCustomiser *m_customiser = nullptr;
    QLabel *m_meta = nullptr;
    QLineEdit *m_search = nullptr;
    QListWidget *m_list = nullptr;
    QLabel *m_chosen = nullptr;
    QKeySequenceEdit *m_recorder = nullptr;
    ui::GhostButton *m_clear = nullptr;
    ui::GhostButton *m_reset = nullptr;
    QWidget *m_warnRow = nullptr;
    QLabel *m_warning = nullptr;
    ui::Button *m_useHere = nullptr;
    ui::GhostButton *m_keepIt = nullptr;
    ui::GhostButton *m_resetAll = nullptr;
    ui::Button *m_done = nullptr;
    QKeySequence m_pendingKey;
    bool m_recording = false;
};

} // namespace kerf
