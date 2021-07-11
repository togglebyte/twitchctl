use fuzzy_filter::FuzzyFilter;
use std::error::Error;
use twitch_api2::{
    helix::{
        channels::{ModifyChannelInformationBody, ModifyChannelInformationRequest},
        points::{
            CreateCustomRewardBody, CreateCustomRewardRequest, CustomReward,
            GetCustomRewardRequest, UpdateCustomRewardBody, UpdateCustomRewardRequest,
        },
        search::{search_categories::Category, SearchCategoriesRequest},
        streams::{
            get_stream_tags::GetStreamTagsRequest,
            replace_stream_tags::{
                ReplaceStreamTags, ReplaceStreamTagsBody, ReplaceStreamTagsRequest,
            },
        },
        tags::{AutoGenerated, GetAllStreamTagsRequest, TwitchTag},
        users::{GetUsersRequest, User},
    },
    twitch_oauth2::{AccessToken, TwitchToken, UserToken},
    types::{CategoryId, Nickname, RewardId, TagId, UserId},
    HelixClient,
};
use twitch_oauth2::client::surf_http_client;

use derivative::Derivative;
use derive_builder::Builder;

use crate::{exit, warning};

#[derive(thiserror::Error, Debug)]
enum ApiError {
    #[error("No user with login `{0}` found.")]
    NoUser(Nickname),
}

pub enum UserIdent {
    UserName(Nickname),
    UserId(UserId),
    None,
}

async fn get_user(token_string: &str) -> Result<UserToken, Box<dyn Error + 'static>> {
    let token = UserToken::from_existing(
        surf_http_client,
        AccessToken::new(token_string.to_string()),
        None,
        None,
    )
    .await?;
    token.validate_token(surf_http_client).await?;

    Ok(token)
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ApiClient<'a> {
    #[derivative(Debug = "ignore")]
    helix_client: HelixClient<'a, surf::Client>,
    token: UserToken,
    user: UserId,
}

impl<'a> ApiClient<'a> {
    pub async fn new(token: &str) -> Result<ApiClient<'a>, Box<dyn Error>> {
        let token = get_user(token).await?;
        Ok(ApiClient {
            helix_client: HelixClient::with_client(surf::Client::new()),
            token: token.clone(),
            user: token.user_id.into(),
        })
    }

    pub fn get_user(&self) -> &str {
        self.token.login.as_ref()
    }

    pub fn get_user_id(&self) -> &UserId {
        &self.user
    }

    pub async fn search_categories(
        &self,
        term: &str,
        max: usize,
    ) -> Result<Option<Vec<Category>>, Box<dyn Error>> {
        // TODO Implement some better filter (only starting with for example) to reduce the number
        // of results for searches

        let req = SearchCategoriesRequest::builder()
            .query(term)
            .first(max.max(1).min(100).to_string())
            .build();
        let res: Vec<Category> = self.helix_client.req_get(req, &self.token).await?.data;
        if res.len() > 0 {
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }
    pub async fn search_category(&self, term: &str) -> Result<Option<Category>, Box<dyn Error>> {
        match self.search_categories(term, 1).await? {
            Some(cs) => Ok(Some(cs[0].clone())),
            None => Ok(None),
        }
    }

    pub async fn get_users(
        &self,
        user_names: &[&Nickname],
        user_ids: &[&UserId],
    ) -> Result<Vec<User>, Box<dyn Error>> {
        let user_names: Vec<Nickname> = user_names.iter().cloned().cloned().collect();
        let user_ids: Vec<UserId> = user_ids.iter().cloned().cloned().collect();
        let req = match (user_names.len(), user_ids.len()) {
            (0, 0) => GetUsersRequest::builder().build(),
            (_, 0) => GetUsersRequest::builder().login(user_names).build(),
            (0, _) => GetUsersRequest::builder().id(user_ids).build(),
            _ => GetUsersRequest::builder()
                .id(user_ids.into())
                .login(user_names.into())
                .build(),
        };

        let res: Vec<User> = self.helix_client.req_get(req, &self.token).await?.data;
        Ok(res)
    }

    pub async fn replace_stream_tags(
        &self,
        broadcaster_id: &UserId,
        tag_ids: Vec<TagId>,
    ) -> Result<ReplaceStreamTags, Box<dyn Error + 'static>> {
        let req = ReplaceStreamTagsRequest::builder()
            .broadcaster_id(broadcaster_id.clone())
            .build();
        let body = ReplaceStreamTagsBody::builder().tag_ids(tag_ids).build();
        let res = self.helix_client.req_put(req, body, &self.token).await?;
        Ok(res.data)
    }

    pub async fn get_stream_tags(&self, id: &UserId) -> Result<Vec<TwitchTag>, Box<dyn Error>> {
        let tag_req = GetStreamTagsRequest::builder()
            .broadcaster_id(id.clone())
            .build();
        let tag_res = self.helix_client.req_get(tag_req, &self.token).await?;
        Ok(tag_res.data)
    }

    pub async fn get_all_tags(&self) -> Result<Vec<TwitchTag>, Box<dyn Error>> {
        let mut tags = vec![];
        let mut pagination = None;
        loop {
            let req = GetAllStreamTagsRequest::builder()
                .after(pagination)
                .first(Some(100))
                .build();
            let mut res = self.helix_client.req_get(req, &self.token).await?;
            tags.append(&mut res.data);
            pagination = res.pagination;
            if pagination == None {
                break;
            }
        }
        Ok(tags)
    }

    pub async fn get_tag_ids_matching(
        &self,
        tags: &[String],
        locale: &str,
    ) -> Result<Vec<TagId>, Box<dyn Error>> {
        let all_tags = self.get_all_tags().await?;

        Ok(tags
            .iter()
            .filter_map(|tag| {
                for tag_obj in all_tags.iter() {
                    match (
                        tag_obj.localization_names.get(locale),
                        tag_obj.localization_names.get("en-us"),
                    ) {
                        (Some(loc_name), _)
                            if loc_name.eq_ignore_ascii_case(tag)
                                && tag_obj.is_auto == AutoGenerated::False =>
                        {
                            return Some(tag_obj.id.clone())
                        }
                        (None, Some(en_name))
                            if en_name.eq_ignore_ascii_case(tag)
                                && tag_obj.is_auto == AutoGenerated::False =>
                        {
                            warning!(
                                "The tag `{}`, has no localized name for `{}`. \
                                Matched English name instead.",
                                en_name,
                                locale
                            );
                            return Some(tag_obj.id.clone());
                        }
                        _ => {}
                    }
                }
                None
            })
            .collect())
    }
    pub async fn get_broadcaster_id(
        &self,
        broadcaster_ident: UserIdent,
    ) -> Result<UserId, Box<dyn Error>> {
        match broadcaster_ident {
            UserIdent::None => Ok(self.get_user_id().clone()),
            UserIdent::UserId(broadcaster_id) => Ok(broadcaster_id),
            UserIdent::UserName(broadcaster_name) => {
                match self.get_users(&[&broadcaster_name], &[]).await {
                    Ok(userlist) => {
                        if userlist.is_empty() {
                            Err(Box::new(ApiError::NoUser(broadcaster_name)))
                        } else {
                            Ok(userlist[0].id.clone())
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }
    pub async fn modify_channel_information(
        &self,
        id: &UserId,
        info: ChannelInfo,
    ) -> Result<(), Box<dyn Error>> {
        let req = ModifyChannelInformationRequest::builder()
            .broadcaster_id(id.clone())
            .build();

        let body = info.to_modify_body();
        self.helix_client.req_patch(req, body, &self.token).await?;
        Ok(())
    }

    pub async fn create_custom_reward(
        &self,
        id: &UserId,
        reward: CreateCustomRewardBody,
    ) -> Result<(), Box<dyn Error>> {
        let req = CreateCustomRewardRequest::builder()
            .broadcaster_id(id.clone())
            .build();

        self.helix_client.req_post(req, reward, &self.token).await?;
        Ok(())
    }

    pub async fn update_custom_reward(
        &self,
        broadcaster_id: &UserId,
        reward_id: &RewardId,
        reward: UpdateCustomRewardBody,
    ) -> Result<(), Box<dyn Error>> {
        let req = UpdateCustomRewardRequest::builder()
            .broadcaster_id(broadcaster_id.clone())
            .id(reward_id.clone())
            .build();
        self.helix_client
            .req_patch(req, reward, &self.token)
            .await?;
        Ok(())
    }

    pub async fn get_rewards(&self, id: &UserId) -> Result<Vec<CustomReward>, Box<dyn Error>> {
        let tag_req = GetCustomRewardRequest::builder()
            .broadcaster_id(id.clone())
            .build();
        let tag_res = self.helix_client.req_get(tag_req, &self.token).await?;
        Ok(tag_res.data)
    }

    pub async fn find_reward(
        &self,
        id: &UserId,
        query: &str,
    ) -> Result<Option<CustomReward>, Box<dyn Error>> {
        let rewards = self.get_rewards(id).await?;

        if let Some(reward) = rewards.iter().find(|r| r.title == query) {
            Ok(Some(reward.clone()))
        } else {
            let query = query.to_lowercase();
            let rewards_ic: Vec<_> = rewards
                .iter()
                .filter(|r| r.title.to_lowercase() == query)
                .collect();
            if rewards_ic.len() == 1 {
                Ok(Some((rewards_ic[0]).clone()))
            } else {
                let query = FuzzyFilter::new(&query);
                let mut rewards = rewards
                    .iter()
                    .filter(|CustomReward { title, .. }| query.matches(&title.to_lowercase()));
                let reward = rewards.next();

                if reward.is_some() && !rewards.next().is_some() {
                    Ok(Some(reward.unwrap().clone()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

#[derive(Default, Builder, Debug)]
#[builder(public, setter(into), default)]
pub struct ChannelInfo {
    title: Option<String>,
    language: Option<String>,
    category: Option<CategoryId>,
}
impl ChannelInfo {
    fn to_modify_body(&self) -> ModifyChannelInformationBody {
        ModifyChannelInformationBody::builder()
            .broadcaster_language(self.language.clone())
            .game_id(self.category.clone())
            .title(self.title.clone())
            .build()
    }
}

pub async fn get_broadcaster_id_or_die(
    client: &ApiClient<'_>,
    broadcaster: Option<Nickname>,
    broadcaster_id: Option<UserId>,
) -> UserId {
    let broadcaster_id = match (broadcaster, broadcaster_id) {
        (_, Some(i)) => client.get_broadcaster_id(UserIdent::UserId(i.into())),
        (Some(b), _) => client.get_broadcaster_id(UserIdent::UserName(b.into())),
        _ => client.get_broadcaster_id(UserIdent::None),
    }
    .await;

    match broadcaster_id {
        Ok(id) => id,
        Err(e) => exit!(1, "{}", e),
    }
}
