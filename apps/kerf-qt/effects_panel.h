#pragma once

// A page of the browser (mockup-spec 8): the catalog's effects, or its
// generators on the Titles page, as a grid of 3 columns of 88 x 50 thumbs
// with names, under a category row (a ghost menu of the catalog's
// categories with live counts, and the item count).
//
// Effects show the selected picture clip (or the program's clip under the
// playhead) through the effect, from effect_previews; hovering a thumb
// scrubs the effect's main parameter by the pointer's x, re-asking for the
// preview at most once per 120 ms. Double-click, Return or a drag to a clip
// adds it. Titles show each generator alone on black, from the generator
// previews.

#include "test_friends.h"

#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QSet>
#include <QSize>
#include <QStringList>
#include <QWidget>

#include <functional>

class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QListWidgetItem;
class QMenu;
class QTimer;

namespace kerf {

namespace ui {
class GhostButton;
}

class EffectsPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Mode { Effects, Titles };

    // The grid (mockup .egrid): pad 8 12, 3 columns of 88, 8 apart, rows 12
    // apart; a thumb is 88 x 50 and its name sits 4 under it on 16.
    static constexpr QSize kCard{88, 50};
    static constexpr int kColumns = 3;
    static constexpr int kColumnGap = 8;
    static constexpr int kRowGap = 12;
    static constexpr int kNameTop = 4;
    static constexpr int kPadX = 12;
    static constexpr int kPadY = 8;
    static constexpr int kCategoryRow = 24;
    // Previews are asked for at twice the thumb's width, for Retina.
    static constexpr int kPreviewWidth = 176;
    // The hover scrub asks again at most this often.
    static constexpr int kScrubIntervalMs = 120;
    // The keys effect_previews requests and events carry.
    static constexpr char kCardKey[] = "card";
    static constexpr char kTitleKey[] = "title";
    static constexpr char kScrubKey[] = "scrub";

    // A row of the category menu.
    struct Category {
        QString key;  // "video", "video:Blur & Sharpen", "audio", "generator", "generator:Generate"
        QString name; // "All video effects", "Blur & Sharpen", ...
        int count = 0;
        bool separatorBefore = false;
    };

    explicit EffectsPanel(QWidget *parent = nullptr, Mode mode = Mode::Effects);
    Mode mode() const { return m_mode; }

    // The catalog from kerf_fx_catalog_all (or kerf_fx_catalog): effects
    // become the Effects page's cards, generators the Titles page's.
    void setCatalog(const QJsonArray &catalog);
    bool hasCatalog() const { return !m_items.isEmpty(); }
    // The clip effects go to; `picture` says whether cards can show it.
    void setClip(const QString &clip, bool picture);
    // The program's picture clip under the playhead: what the cards show
    // when no picture clip is selected.
    void setProgramClip(const QString &clip);
    // The clip the previews are of: the selected picture clip, else the
    // program's; empty when there is neither.
    QString previewClip() const;
    // Show the search field and put the cursor in it.
    void focusSearch();
    // Deprecated and a no-op: an effect_preview without its key may answer
    // another region's request. The keyed call below takes every answer.
    void setPreview(const QString &clip, const QString &fx, const QImage &image);
    // The keyed effect_preview. The Effects page takes kCardKey (a resting
    // thumb) and kScrubKey (the hover scrub's frame), the Titles page only
    // kTitleKey; every other key is dropped.
    void setPreview(const QString &clip, const QString &fx, const QString &key, const QImage &image);
    // Whether `key` is one this page asks with.
    bool ownsPreviewKey(const QString &key) const;
    // Whether a card can be added here; `reason` says why not (the cards'
    // tips carry it, and activating one does nothing).
    void setAddAvailable(bool available, const QString &reason = {});
    bool addAvailable() const { return m_addAvailable; }

    QList<Category> categories() const { return m_categories; }
    QString category() const;
    void setCategory(const QString &key);
    // The cards shown (category and search), as the count says.
    int shownCount() const;
    // The category menu as the ghost button opens it.
    QMenu *categoryMenu();

    // The hover scrub: the pointer at `fraction` (0..1) across `fx`'s thumb,
    // and off it again.
    void scrubAt(const QString &fx, double fraction);
    void endScrub();
    QString scrubbing() const { return m_scrubFx; }
    // The parameter a scrub moves: {name, min, max}, or empty when the
    // effect has none (an amount, a mix, a radius, else its first ranged number).
    static QJsonObject mainParam(const QJsonObject &fx);
    // Milliseconds from any fixed start, for the scrub's throttle; tests set one.
    void setClock(std::function<qint64()> clock);
    // Ask for the resting thumbs the shown cards lack, now.
    void requestPreviews();
    // Ask soon, as a change of clip, category or page does: only while the
    // page is on screen and the gate (an engine to answer) allows it.
    void schedulePreviews();
    // Whether the panel asks on its own (a bridge is there); requests a
    // test or a person makes go regardless. Open by default.
    void setPreviewGate(std::function<bool()> gate);

signals:
    void addRequested(const QString &fx);
    // Previews wanted for the cards shown, on the clip ("" for generators).
    void previewsWanted(const QString &clip, const QStringList &effects);
    // A hover scrub's preview: `fx` with `params` on `clip`.
    void scrubWanted(const QString &clip, const QString &fx, const QJsonObject &params);

protected:
    void paintEvent(QPaintEvent *event) override;
    void showEvent(QShowEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void refilter();
    void add(QListWidgetItem *item);
    QStringList shownEffects() const;
    void rebuildCategories();
    void resetPreviews();
    void setCardImage(QListWidgetItem *item, const QImage &image);
    void sendScrub();
    void updateTips();
    qint64 now() const;

    Mode m_mode;
    // The category in effect: its items hold each row's key (the ghost
    // button's menu is its face).
    QComboBox *m_category = nullptr;
    ui::GhostButton *m_categoryButton = nullptr;
    QLabel *m_count = nullptr;
    QWidget *m_searchRow = nullptr;
    QLineEdit *m_search = nullptr;
    QListWidget *m_cards = nullptr;
    // The empty state: no catalog, or nothing matches.
    QLabel *m_hint = nullptr;
    QTimer *m_previewTimer = nullptr;
    QTimer *m_scrubTimer = nullptr;
    QString m_clip;
    QString m_programClip;
    bool m_picture = false;
    bool m_addAvailable = true;
    QString m_addReason;
    QHash<QString, QListWidgetItem *> m_items;
    QHash<QString, QJsonObject> m_fx;
    QHash<QString, QImage> m_base;
    QList<Category> m_categories;
    // The hover scrub: the card, the value waiting to go, when one last went,
    // and the effects with an answer still due.
    QString m_scrubFx;
    double m_scrubFraction = 0;
    qint64 m_lastScrub = -1'000'000;
    QSet<QString> m_scrubDue;
    std::function<qint64()> m_clock;
    std::function<bool()> m_gate;
};

} // namespace kerf
