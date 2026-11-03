#pragma once

#include "test_friends.h"

#include <QHash>
#include <QJsonArray>
#include <QWidget>

class QComboBox;
class QImage;
class QLabel;
class QLineEdit;
class QListWidget;
class QListWidgetItem;
class QTimer;

namespace kerf {

// Effects: the catalog's picture effects as cards, narrowed by category and
// words, each showing the selected clip's picture through the effect.
// Double-click or Return adds the effect to the clip.
class EffectsPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit EffectsPanel(QWidget *parent = nullptr);

    // The catalog from kerf_fx_catalog; its effects become cards.
    void setCatalog(const QJsonArray &catalog);
    // The clip effects go to; `picture` says whether cards can show it.
    void setClip(const QString &clip, bool picture);
    // Put the cursor in the search field (Add effect in the Inspector).
    void focusSearch();
    // How `fx` looks on `clip`.
    void setPreview(const QString &clip, const QString &fx, const QImage &image);

signals:
    void addRequested(const QString &fx);
    // Previews wanted for the cards shown, on the clip.
    void previewsWanted(const QString &clip, const QStringList &effects);

private:
    void refilter();
    void requestPreviews();
    void add(QListWidgetItem *item);
    QStringList shownEffects() const;

    QComboBox *m_category = nullptr;
    QLineEdit *m_search = nullptr;
    QListWidget *m_cards = nullptr;
    QLabel *m_hint = nullptr;
    QTimer *m_previewTimer = nullptr;
    QString m_clip;
    bool m_picture = false;
    QHash<QString, QListWidgetItem *> m_items;
};

} // namespace kerf
